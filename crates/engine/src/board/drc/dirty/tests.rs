//! Dirty-region tests built through shared production commands and checked against a brute-force
//! neighbor oracle.

use super::*;
use crate::{
    Session,
    board::track::{
        ADD_TRACKS, ADD_VIA, DEFAULT_VIA_DIAMETER_NM, DEFAULT_VIA_DRILL_NM, DEFAULT_WIDTH_NM, DELETE_TRACKS, MOVE_TRACKS, SET_VIA_SIZE,
        tests::{copper_layers, routed_fixture, view},
    },
    net_classes,
    nets::{Anchor, AnchorKind, ClassTarget, SET_CLASSES},
    persistence::encode_documents,
    schematic::Point,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

fn add_via(session: &mut Session, at: Point) -> Uuid {
    let before: BTreeSet<Uuid> = view(session).vias().iter().map(|via| via.id).collect();
    apply(session, ADD_VIA, json!({"at": at, "via_diameter_nm": DEFAULT_VIA_DIAMETER_NM, "via_drill_nm": DEFAULT_VIA_DRILL_NM}));
    view(session).vias().iter().find(|via| !before.contains(&via.id)).unwrap().id
}

fn add_track(session: &mut Session, layer: Uuid, start: Point, end: Point) -> Uuid {
    let before: BTreeSet<Uuid> = view(session).tracks().iter().map(|track| track.id).collect();
    apply(session, ADD_TRACKS, json!({"layer": layer, "width_nm": DEFAULT_WIDTH_NM, "points": [start, end]}));
    view(session).tracks().iter().find(|track| !before.contains(&track.id)).unwrap().id
}

fn query(session: &Session) -> ClearanceQuery {
    session.clearance_query_with_rules(view(session).target, &json!({})).unwrap()
}

fn index_by_name(query: &ClearanceQuery, name: &str) -> usize {
    query.objects().iter().position(|obj| obj.name == name).unwrap()
}

fn share_layer(left: &[Uuid], right: &[Uuid]) -> bool {
    left.iter().any(|layer| right.contains(layer))
}

/// Independent brute-force neighbor oracle that scans every snapshot object linearly without
/// consulting any spatial index.
fn brute_force_dirty_indices(after: &ClearanceQuery, delta: &EditDelta) -> Vec<usize> {
    let max_clearance = after.authority().clearance_nm() as f64;
    let mut expected = BTreeSet::new();
    for change in delta.changes() {
        if let Some(after_idx) = change.after_index {
            expected.insert(after_idx);
        }
        for footprint in change.before.iter().chain(change.after.iter()) {
            for (idx, object) in after.objects().iter().enumerate() {
                if share_layer(&object.layers, &footprint.layers) && object.contour.gap(&footprint.contour) <= max_clearance {
                    expected.insert(idx);
                }
            }
        }
    }
    expected.into_iter().collect()
}

fn assign_power_class(session: &mut Session, clearance_nm: i64) {
    let before: BTreeSet<Uuid> = session.project().unwrap().net_classes().definitions.iter().map(|class| class.id).collect();
    apply(session, net_classes::CREATE, json!({"name": "Power", "rules": {"clearance_nm": clearance_nm}}));
    let class_id = session.project().unwrap().net_classes().definitions.iter().find(|class| !before.contains(&class.id)).unwrap().id;
    let project = session.project().unwrap();
    let component = *project.components.keys().next().unwrap();
    let anchor = Anchor {
        kind: AnchorKind::Terminal,
        project: project.id,
        scope: project.net_scope(),
        occurrence: Uuid::nil(),
        object: component,
        terminal: "1".into(),
    };
    apply(session, SET_CLASSES, json!({"target": ClassTarget::Derived { anchor }, "classes": [class_id]}));
}

/// Moving one via yields exactly the via plus every object within the largest clearance rule of
/// its previous or new footprint, excluding objects just outside or only inside the bounding box.
#[test]
fn moving_one_via_matches_brute_force_neighbors_within_clearance() {
    let mut s = routed_fixture();
    let layers = copper_layers(&s);
    let front = layers[0];

    let clearance = query(&s).authority().clearance_nm();
    assert!(clearance > 0);
    let r = DEFAULT_VIA_DIAMETER_NM / 2;
    let old_at = Point { x_nm: 100_000_000, y_nm: 100_000_000 };
    let new_at = Point { x_nm: 110_000_000, y_nm: 100_000_000 };

    let moved_via = add_via(&mut s, old_at);
    let near_old_via = add_via(&mut s, Point { x_nm: old_at.x_nm, y_nm: old_at.y_nm + 2 * r + clearance / 2 });
    let boundary_new_via = add_via(&mut s, Point { x_nm: new_at.x_nm + 2 * r + clearance, y_nm: new_at.y_nm });
    let just_outside_new_via = add_via(&mut s, Point { x_nm: new_at.x_nm - (2 * r + clearance + 1), y_nm: new_at.y_nm });
    // Inside the expanded AABB along both axes (dx = dy = 2r + c), yet Euclidean gap exceeds c.
    let diagonal_aabb_via = add_via(&mut s, Point { x_nm: new_at.x_nm + 2 * r + clearance, y_nm: new_at.y_nm + 2 * r + clearance });
    let far_via = add_via(&mut s, Point { x_nm: 140_000_000, y_nm: 100_000_000 });
    let track_y = new_at.y_nm - (r + DEFAULT_WIDTH_NM / 2 + clearance / 2);
    let near_new_track = add_track(&mut s, front, Point { x_nm: new_at.x_nm, y_nm: track_y }, Point { x_nm: new_at.x_nm + 2_000_000, y_nm: track_y });

    let before = query(&s);
    apply(&mut s, MOVE_TRACKS, json!({"vias": [moved_via], "dx_nm": new_at.x_nm - old_at.x_nm, "dy_nm": 0}));
    let saved_before = encode_documents(s.snapshot().unwrap()).unwrap();
    let rev_before = s.revision();
    let after = query(&s);

    let delta = EditDelta::between(&before, &after).unwrap();
    assert_eq!(delta.target(), after.authority().target());
    assert_eq!(delta.from_revision(), before.authority().revision());
    assert_eq!(delta.to_revision(), after.authority().revision());
    assert_eq!(delta.changes().len(), 1);
    assert_eq!(delta.changes()[0].name, moved_via.to_string());
    assert_eq!(delta.changes()[0].change, ChangeKind::Modified);

    let dirty = DirtyRegion::from_delta(&after, &delta).unwrap();
    let expected = brute_force_dirty_indices(&after, &delta);
    assert_eq!(dirty.target(), after.authority().target());
    assert_eq!(dirty.from_revision(), before.authority().revision());
    assert_eq!(dirty.to_revision(), after.authority().revision());
    assert_eq!(dirty.dirty_indices(), expected.as_slice());
    assert_eq!(dirty.dirty_names().len(), dirty.dirty_indices().len());
    assert!(dirty.dirty_names().contains(&moved_via.to_string()));
    assert_eq!(dirty.max_clearance_nm(), clearance);
    assert_eq!(dirty.changed_indices(), &[index_by_name(&after, &moved_via.to_string())]);
    assert!(dirty.removed_names().is_empty());

    assert!(dirty.contains_index(index_by_name(&after, &moved_via.to_string())));
    assert!(dirty.contains_index(index_by_name(&after, &near_old_via.to_string())));
    assert!(dirty.contains_index(index_by_name(&after, &boundary_new_via.to_string())));
    assert!(dirty.contains_index(index_by_name(&after, &near_new_track.to_string())));
    assert!(!dirty.contains_index(index_by_name(&after, &just_outside_new_via.to_string())));
    assert!(!dirty.contains_index(index_by_name(&after, &diagonal_aabb_via.to_string())));
    assert!(!dirty.contains_index(index_by_name(&after, &far_via.to_string())));

    // Computing deltas and dirty regions is strictly read-only.
    assert_eq!(s.revision(), rev_before);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved_before);
}

/// When a net class defines a larger clearance rule than the board default, the dirty region
/// expands to that largest rule.
#[test]
fn dirty_region_expands_to_largest_net_class_clearance_rule() {
    let mut s = routed_fixture();
    let default_clearance = query(&s).authority().clearance_nm();
    let class_clearance = default_clearance + 300_000;
    assign_power_class(&mut s, class_clearance);

    let r = DEFAULT_VIA_DIAMETER_NM / 2;
    let old_at = Point { x_nm: 100_000_000, y_nm: 100_000_000 };
    let new_at = Point { x_nm: 112_000_000, y_nm: 100_000_000 };
    let moved_via = add_via(&mut s, old_at);
    // Beyond board default clearance, inside the net-class clearance rule.
    let inside_class_via = add_via(&mut s, Point { x_nm: new_at.x_nm + 2 * r + default_clearance + 150_000, y_nm: new_at.y_nm });
    // Just outside the net-class clearance rule.
    let outside_class_via = add_via(&mut s, Point { x_nm: new_at.x_nm - (2 * r + class_clearance + 1), y_nm: new_at.y_nm });

    let before = query(&s);
    assert_eq!(before.authority().clearance_nm(), class_clearance);
    apply(&mut s, MOVE_TRACKS, json!({"vias": [moved_via], "dx_nm": new_at.x_nm - old_at.x_nm, "dy_nm": 0}));
    let after = query(&s);

    let delta = EditDelta::between(&before, &after).unwrap();
    let dirty = DirtyRegion::between(&before, &after).unwrap();
    assert_eq!(dirty.max_clearance_nm(), class_clearance);
    assert_eq!(dirty.dirty_indices(), brute_force_dirty_indices(&after, &delta).as_slice());
    assert!(dirty.contains_index(index_by_name(&after, &inside_class_via.to_string())));
    assert!(!dirty.contains_index(index_by_name(&after, &outside_class_via.to_string())));
}

/// Above the packed-index threshold (128 objects), spatial-index dirty lookup still matches the
/// linear brute-force neighbor list exactly.
#[test]
fn packed_spatial_index_dirty_region_matches_brute_force() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let mut track_ids = Vec::new();
    for row in 0..14_i64 {
        for col in 0..10_i64 {
            let x = 50_000_000 + col * 3_000_000;
            let y = 50_000_000 + row * 1_500_000;
            track_ids.push(add_track(&mut s, front, Point { x_nm: x, y_nm: y }, Point { x_nm: x + 1_200_000, y_nm: y }));
        }
    }
    let old_at = Point { x_nm: 45_000_000, y_nm: 50_000_000 };
    let target_at = Point { x_nm: 56_500_000, y_nm: 53_500_000 };
    let moved_via = add_via(&mut s, old_at);

    let before = query(&s);
    assert!(before.objects().len() >= 140, "fixture must cross the packed RTree threshold");
    apply(&mut s, MOVE_TRACKS, json!({"vias": [moved_via], "dx_nm": target_at.x_nm - old_at.x_nm, "dy_nm": target_at.y_nm - old_at.y_nm}));
    let after = query(&s);

    let delta = EditDelta::between(&before, &after).unwrap();
    let dirty = DirtyRegion::from_delta(&after, &delta).unwrap();
    let expected = brute_force_dirty_indices(&after, &delta);
    assert_eq!(dirty.dirty_indices(), expected.as_slice());
    assert!(dirty.dirty_indices().len() >= 2, "moved via should dirty at least one nearby track");
    assert!(dirty.dirty_indices().len() < track_ids.len() / 2, "dirty set stays localized");
}

/// A single-layer track edit only dirties neighbors sharing that copper layer, plus through vias.
#[test]
fn single_layer_track_edit_respects_layer_isolation() {
    let mut s = routed_fixture();
    let layers = copper_layers(&s);
    let (front, back) = (layers[0], layers[1]);
    let clearance = query(&s).authority().clearance_nm();

    let moved_track = add_track(&mut s, front, Point { x_nm: 80_000_000, y_nm: 80_000_000 }, Point { x_nm: 82_000_000, y_nm: 80_000_000 });
    let dest_x = 90_000_000;
    let dest_y = 80_000_000;
    let back_under_dest = add_track(&mut s, back, Point { x_nm: dest_x, y_nm: dest_y }, Point { x_nm: dest_x + 2_000_000, y_nm: dest_y });
    let front_near_dest = add_track(
        &mut s,
        front,
        Point { x_nm: dest_x, y_nm: dest_y + DEFAULT_WIDTH_NM + clearance / 2 },
        Point { x_nm: dest_x + 2_000_000, y_nm: dest_y + DEFAULT_WIDTH_NM + clearance / 2 },
    );
    let through_via_near_dest =
        add_via(&mut s, Point { x_nm: dest_x, y_nm: dest_y - (DEFAULT_WIDTH_NM / 2 + DEFAULT_VIA_DIAMETER_NM / 2 + clearance / 2) });

    let before = query(&s);
    apply(&mut s, MOVE_TRACKS, json!({"tracks": [moved_track], "dx_nm": dest_x - 80_000_000, "dy_nm": 0}));
    let after = query(&s);

    let delta = EditDelta::between(&before, &after).unwrap();
    let dirty = DirtyRegion::from_delta(&after, &delta).unwrap();
    assert_eq!(dirty.dirty_indices(), brute_force_dirty_indices(&after, &delta).as_slice());
    assert!(dirty.contains_index(index_by_name(&after, &moved_track.to_string())));
    assert!(dirty.contains_index(index_by_name(&after, &front_near_dest.to_string())));
    assert!(dirty.contains_index(index_by_name(&after, &through_via_near_dest.to_string())));
    assert!(!dirty.contains_index(index_by_name(&after, &back_under_dest.to_string())));
}

/// Adding, editing drill size, deleting a via, and comparing identical snapshots each yield exact
/// edit deltas and dirty sets.
#[test]
fn add_delete_drill_edit_and_noop_match_brute_force() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let clearance = query(&s).authority().clearance_nm();
    let anchor = Point { x_nm: 100_000_000, y_nm: 100_000_000 };
    let neighbor_y = anchor.y_nm + DEFAULT_VIA_DIAMETER_NM / 2 + DEFAULT_WIDTH_NM / 2 + clearance / 2;
    let neighbor = add_track(
        &mut s,
        front,
        Point { x_nm: anchor.x_nm - 1_000_000, y_nm: neighbor_y },
        Point { x_nm: anchor.x_nm + 1_000_000, y_nm: neighbor_y },
    );

    // No-op comparison produces an empty delta and empty dirty region.
    let q0 = query(&s);
    let noop_delta = EditDelta::between(&q0, &q0).unwrap();
    assert!(noop_delta.is_empty());
    let noop_dirty = DirtyRegion::from_delta(&q0, &noop_delta).unwrap();
    assert!(noop_dirty.is_empty());

    // Adding a via marks the via and its nearby track dirty.
    let via_id = add_via(&mut s, anchor);
    let q1 = query(&s);
    let add_delta = EditDelta::between(&q0, &q1).unwrap();
    assert_eq!(add_delta.changes().len(), 1);
    assert_eq!(add_delta.changes()[0].change, ChangeKind::Added);
    let add_dirty = DirtyRegion::from_delta(&q1, &add_delta).unwrap();
    assert_eq!(add_dirty.dirty_indices(), brute_force_dirty_indices(&q1, &add_delta).as_slice());
    assert!(add_dirty.contains_index(index_by_name(&q1, &via_id.to_string())));
    assert!(add_dirty.contains_index(index_by_name(&q1, &neighbor.to_string())));

    // Editing only via drill size (same outer contour) still invalidates the via and its neighbors.
    apply(&mut s, SET_VIA_SIZE, json!({"vias": [via_id], "via_diameter_nm": DEFAULT_VIA_DIAMETER_NM, "via_drill_nm": 350_000}));
    let q2 = query(&s);
    let drill_delta = EditDelta::between(&q1, &q2).unwrap();
    assert_eq!(drill_delta.changes().len(), 1);
    assert_eq!(drill_delta.changes()[0].change, ChangeKind::Modified);
    let drill_dirty = DirtyRegion::from_delta(&q2, &drill_delta).unwrap();
    assert_eq!(drill_dirty.dirty_indices(), brute_force_dirty_indices(&q2, &drill_delta).as_slice());
    assert!(drill_dirty.contains_index(index_by_name(&q2, &via_id.to_string())));
    assert!(drill_dirty.contains_index(index_by_name(&q2, &neighbor.to_string())));

    // Deleting the via records its removal and dirties the vacated neighbor.
    apply(&mut s, DELETE_TRACKS, json!({"vias": [via_id]}));
    let q3 = query(&s);
    let del_delta = EditDelta::between(&q2, &q3).unwrap();
    assert_eq!(del_delta.changes().len(), 1);
    assert_eq!(del_delta.changes()[0].change, ChangeKind::Removed);
    let del_dirty = DirtyRegion::from_delta(&q3, &del_delta).unwrap();
    assert_eq!(del_dirty.dirty_indices(), brute_force_dirty_indices(&q3, &del_delta).as_slice());
    assert!(del_dirty.changed_indices().is_empty());
    assert_eq!(del_dirty.removed_names(), &[via_id.to_string()]);
    assert!(del_dirty.contains_index(index_by_name(&q3, &neighbor.to_string())));
}

/// Cross-board comparisons and stale target snapshots are explicitly refused.
#[test]
fn refuses_mismatched_board_target_and_stale_revision() {
    let mut first = routed_fixture();
    let second = routed_fixture();

    let first_before = query(&first);
    let second_query = query(&second);
    assert!(matches!(EditDelta::between(&first_before, &second_query), Err(CommandError::InvalidParams(_))));

    let via_id = add_via(&mut first, Point { x_nm: 100_000_000, y_nm: 100_000_000 });
    let first_after = query(&first);
    let delta = EditDelta::between(&first_before, &first_after).unwrap();
    assert!(matches!(DirtyRegion::from_delta(&second_query, &delta), Err(CommandError::InvalidParams(_))));

    apply(&mut first, MOVE_TRACKS, json!({"vias": [via_id], "dx_nm": 1_000_000, "dy_nm": 0}));
    let first_later = query(&first);
    assert!(matches!(DirtyRegion::from_delta(&first_later, &delta), Err(CommandError::InvalidParams(_))));
}
