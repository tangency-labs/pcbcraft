//! Bulk net property edits retain per-net values on untouched channels.

use super::*;
use crate::{
    net_classes,
    nets::{ClassTarget, SET_CLASSES, SET_PROPERTIES},
};

/// Apply width and class membership in one step without flattening the original colors or classes.
#[test]
fn bulk_changes_are_sparse_and_undo_together() {
    let (mut session, pins) = circuit(4, false);
    connect(&mut session, &pins[0], &pins[1]);
    connect(&mut session, &pins[2], &pins[3]);
    let project = session.project().unwrap();
    let targets: Vec<_> = project
        .wires()
        .iter()
        .map(|wire| project.net_class_target_for_endpoint(project.sheet_target(), wire.endpoints()[0]).unwrap().unwrap())
        .collect();
    for name in ["Signal", "Other"] {
        apply(&mut session, net_classes::CREATE, json!({"name":name,"rules":{}}));
    }
    let class = |name: &str| session.project().unwrap().net_classes().definitions.iter().find(|class| class.name == name).unwrap().id;
    let (signal, other) = (class("Signal"), class("Other"));
    apply(&mut session, SET_CLASSES, json!({"target":targets[0],"classes":[other]}));
    apply(&mut session, SET_APPEARANCE, json!({"target":targets[0],"appearance":{"color":[10,20,30]}}));
    apply(&mut session, SET_APPEARANCE, json!({"target":targets[1],"appearance":{"color":[100,110,120],"width_cp":260}}));

    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    let params = json!({"targets":targets,"changes":{"appearance":{"width_cp":80},"add_classes":[signal]}});
    apply(&mut session, SET_PROPERTIES, params.clone());
    assert_eq!(session.revision(), revision + 1);
    let values = session.project().unwrap().net_property_values(&targets).unwrap();
    assert_eq!(values[0].appearance, Some(Appearance { color: Some([10, 20, 30]), width_cp: Some(80) }));
    assert_eq!(values[1].appearance, Some(Appearance { color: Some([100, 110, 120]), width_cp: Some(80) }));
    assert_eq!(values[0].classes, [signal, other].into());
    assert_eq!(values[1].classes, [signal].into());
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    apply(&mut session, SET_PROPERTIES, params);
    assert_eq!(session.revision(), revision + 1);

    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);

    apply(&mut session, SET_PROPERTIES, json!({"targets":targets,"changes":{"appearance":{"clear":["color"]},"remove_classes":[signal]}}));
    let values = session.project().unwrap().net_property_values(&targets).unwrap();
    assert_eq!(values[0].classes, [other].into());
    assert!(values[1].classes.is_empty());
    assert!(values.iter().all(|value| value.appearance == Some(Appearance { color: None, width_cp: Some(80) })));
}

/// Reject a mixed valid/foreign batch and two spellings of one net without changing any of them.
#[test]
fn bulk_rejects_invalid_and_aliased_targets_atomically() {
    let (mut session, pins) = circuit(2, false);
    connect(&mut session, &pins[0], &pins[1]);
    let project = session.project().unwrap();
    let endpoint = project.wires()[0].endpoints()[0];
    let target = project.net_class_target_for_endpoint(project.sheet_target(), endpoint).unwrap().unwrap();
    let id = promote(&mut session, &pins[0]);
    let project = session.project().unwrap();
    let persistent = ClassTarget::Persistent { project: project.id(), scope: project.net_scope(), net: id };
    let foreign = ClassTarget::Persistent { project: Uuid::from_u128(71), scope: project.net_scope(), net: id };
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    for invalid in [foreign, persistent] {
        let params = json!({"expected_revision":revision,"targets":[target,invalid],"changes":{"appearance":{"width_cp":80}}});
        assert!(session.apply(SET_PROPERTIES, &params).is_err());
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
}
