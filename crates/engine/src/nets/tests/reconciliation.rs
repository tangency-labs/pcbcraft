//! Appearance overrides survive topology reconciliation through successor records.

use super::*;

/// Author one net-wide override on a promoted identity through the normal command path.
fn set_appearance(session: &mut Session, id: Uuid, appearance: Appearance) {
    let project = session.project().unwrap();
    let target = serde_json::to_value(ClassTarget::Persistent { project: project.id(), scope: project.net_scope(), net: id }).unwrap();
    apply(session, SET_APPEARANCE, json!({"target":target,"appearance":serde_json::to_value(appearance).unwrap()}));
}

/// A split keeps the source override on both the retained identity and the fresh successor, so
/// every remaining wire still paints with it even though the source network no longer exists.
#[test]
fn split_inherits_the_source_override_on_every_successor() {
    let (mut s, pins) = circuit(4, true);
    let base = connect(&mut s, &pins[0], &pins[1]);
    let middle = connect(&mut s, &pins[1], &pins[2]);
    let tail = connect(&mut s, &pins[2], &pins[3]);
    let original = promote(&mut s, &pins[0]);
    let expected = Appearance { color: Some([10, 20, 30]), width_cp: Some(300) };
    set_appearance(&mut s, original, expected);

    apply(&mut s, crate::DELETE_WIRE, json!({"wire":middle}));
    let child = id_for(&s, &pins[2]);
    assert_ne!(child, original);
    assert_eq!(id_for(&s, &pins[0]), original);
    for id in [original, child] {
        assert_eq!(s.project().unwrap().net_registry.record(id).unwrap().appearance, Some(expected));
        assert!(matches!(s.project().unwrap().net_registry.record(id).unwrap().state, State::Active {}));
    }
    let painted = s.project().unwrap().wire_appearances().unwrap();
    assert_eq!(painted.get(&base), Some(&expected));
    assert_eq!(painted.get(&tail), Some(&expected));
    assert_eq!(painted.len(), 2);
    round_trip(&s);
}

/// Conflicting merge overrides resolve to the contributing source with the lowest UUID, while
/// retired sources keep their own saved appearance as historical evidence.
#[test]
fn merge_conflicting_overrides_follow_the_lowest_source_uuid() {
    let (mut s, pins) = circuit(4, false);
    connect(&mut s, &pins[0], &pins[1]);
    let first = promote(&mut s, &pins[0]);
    let a = Appearance { color: Some([1, 2, 3]), width_cp: None };
    set_appearance(&mut s, first, a);
    connect(&mut s, &pins[2], &pins[3]);
    let second = promote(&mut s, &pins[2]);
    let b = Appearance { color: None, width_cp: Some(400) };
    set_appearance(&mut s, second, b);
    assert_ne!(first, second);

    connect(&mut s, &pins[1], &pins[2]);
    let merged = id_for(&s, &pins[0]);
    assert_ne!(merged, first);
    assert_ne!(merged, second);
    let expected = if first < second { a } else { b };
    let registry = s.project().unwrap().net_registry.clone();
    assert_eq!(registry.record(merged).unwrap().appearance, Some(expected));
    assert_eq!(registry.record(first).unwrap().appearance, Some(a));
    assert_eq!(registry.record(second).unwrap().appearance, Some(b));
    for id in [first, second] {
        assert!(matches!(registry.record(id).unwrap().state, State::Retired { reason: Retirement::Merged, .. }));
    }
    round_trip(&s);
}

/// Split appearance is restored exactly with undo and redo, including the painted wires.
#[test]
fn split_appearance_restores_on_undo_and_redo() {
    let (mut s, pins) = circuit(4, true);
    connect(&mut s, &pins[0], &pins[1]);
    let middle = connect(&mut s, &pins[1], &pins[2]);
    connect(&mut s, &pins[2], &pins[3]);
    let original = promote(&mut s, &pins[0]);
    let expected = Appearance { color: Some([7, 8, 9]), width_cp: Some(175) };
    set_appearance(&mut s, original, expected);

    let before = s.project().unwrap().net_registry.clone();
    let before_wires = s.project().unwrap().wire_appearances().unwrap();
    assert_eq!(before_wires.len(), 3);

    apply(&mut s, crate::DELETE_WIRE, json!({"wire":middle}));
    let after = s.project().unwrap().net_registry.clone();
    let after_wires = s.project().unwrap().wire_appearances().unwrap();
    assert!(after.records.len() > before.records.len());
    assert_eq!(after_wires.len(), 2);

    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(s.project().unwrap().net_registry, before);
    assert_eq!(s.project().unwrap().wire_appearances().unwrap(), before_wires);
    assert_eq!(s.project().unwrap().net_registry.record(original).unwrap().appearance, Some(expected));

    apply(&mut s, "edit.redo", json!({}));
    assert_eq!(s.project().unwrap().net_registry, after);
    assert_eq!(s.project().unwrap().wire_appearances().unwrap(), after_wires);
    let child = id_for(&s, &pins[2]);
    assert_eq!(s.project().unwrap().net_registry.record(child).unwrap().appearance, Some(expected));
}

/// Sources without an override leave successors unstyled; a single override among contributing
/// sources is still carried forward without depending on the unstyled source.
#[test]
fn sources_without_overrides_leave_successors_unstyled() {
    let (mut s, pins) = circuit(4, false);
    connect(&mut s, &pins[0], &pins[1]);
    promote(&mut s, &pins[0]);
    connect(&mut s, &pins[2], &pins[3]);
    promote(&mut s, &pins[2]);
    connect(&mut s, &pins[1], &pins[2]);
    let merged = id_for(&s, &pins[0]);
    assert_eq!(s.project().unwrap().net_registry.record(merged).unwrap().appearance, None);
    assert!(s.project().unwrap().wire_appearances().unwrap().is_empty());

    let (mut s, pins) = circuit(4, false);
    connect(&mut s, &pins[0], &pins[1]);
    let styled = promote(&mut s, &pins[0]);
    let only = Appearance { color: Some([40, 50, 60]), width_cp: None };
    set_appearance(&mut s, styled, only);
    connect(&mut s, &pins[2], &pins[3]);
    promote(&mut s, &pins[2]);
    connect(&mut s, &pins[1], &pins[2]);
    let merged = id_for(&s, &pins[0]);
    assert_eq!(s.project().unwrap().net_registry.record(merged).unwrap().appearance, Some(only));
}
