use super::{INSPECT, SET};
use crate::{
    CommandError, Session, net_classes, net_labels,
    nets::{AnchorKind, NetAddress, State},
};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// Author every fixture object using the existing revision-checked Session path.
fn apply(session: &mut Session, command: &str, mut args: Value) {
    args["expected_revision"] = json!(session.revision());
    session.apply(command, &args).unwrap();
}

/// Add a named singleton and retrieve its already-authored persistent net identity.
fn named_net(session: &mut Session, name: &str, x_nm: i64) -> (NetAddress, String) {
    apply(session, net_labels::ADD, json!({"name":name,"style":"local","x_nm":x_nm,"y_nm":0}));
    let project = session.project().unwrap();
    let label = project.net_labels().iter().find(|label| label.name == name).unwrap();
    let net = project.net_registry.records.iter().find(|r| r.primary.kind == AnchorKind::Label && r.primary.object == label.id).unwrap().id;
    (NetAddress { project: project.id(), scope: project.net_scope(), net }, format!("net-label-{}", label.id))
}

/// Supply project-qualified targets and the revision of their current authoring snapshot.
fn request(session: &Session, target: &Value, intent: &Value) -> Value {
    json!({"version":1,"expected_revision":session.revision(),"target":target,"intent":intent})
}

/// Query authored data through the shared command adapter rather than inspecting private fields.
fn inspect(session: &mut Session, target: &Value) -> Value {
    session.execute(INSPECT, &json!({"version":1,"target":target})).unwrap()["intent"].clone()
}

/// A synthetic named net and existing catalog class retain exact values independently. Preview
/// and repeated writes do not publish, while one Undo/Redo restores the complete changed intent.
#[test]
fn authored_electrical_intent_set_read_clear_and_undo() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    let (address, _) = named_net(&mut session, "TEST_SUPPLY", 0);
    let (reference, _) = named_net(&mut session, "TEST_RETURN", 25_400_000);
    apply(&mut session, net_classes::CREATE, json!({"name":"Test class","rules":{}}));
    let class = session.project().unwrap().net_classes().definitions[0].id;
    let net_target = json!({"kind":"net","address":address});
    let class_target = json!({"kind":"net_class","project":address.project,"class":class});
    let unset = json!({"working_voltage":null,"maximum_continuous_current":null,"allowed_temperature_rise":null,"isolation_group":null});
    assert_eq!(inspect(&mut session, &net_target), unset);
    assert_eq!(inspect(&mut session, &class_target), unset);
    let values = json!({
        "working_voltage":{"magnitude":{"value":"0","unit":"V"},"basis":"dc","reference":{"kind":"net","address":reference}},
        "maximum_continuous_current":{"value":"0","unit":"A"},
        "allowed_temperature_rise":{"value":"0.000000000000000001","unit":"K"},
        "isolation_group":"Test primary"
    });
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let undo = session.undo.len();
    let args = request(&session, &net_target, &values);
    let mut preview = args.clone();
    preview["dry_run"] = json!(true);
    assert_eq!(session.apply(SET, &preview).unwrap(), revision);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.undo.len(), undo);
    assert_eq!(inspect(&mut session, &net_target), unset);

    assert_eq!(session.apply(SET, &args).unwrap(), revision + 1);
    assert_eq!(session.undo.len(), undo + 1);
    assert_eq!(inspect(&mut session, &net_target), values);
    assert_eq!(inspect(&mut session, &class_target), unset);
    let edited = session.snapshot().unwrap();
    let repeated = request(&session, &net_target, &values);
    assert_eq!(session.apply(SET, &repeated).unwrap(), revision + 1);
    assert!(Arc::ptr_eq(&edited, &session.snapshot().unwrap()));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(inspect(&mut session, &net_target), unset);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(inspect(&mut session, &net_target), values);

    let class_values = json!({"working_voltage":{"magnitude":{"value":"36.5","unit":"V"},"basis":"peak_ac","reference":{"kind":"earth"}},
        "maximum_continuous_current":{"value":"3","unit":"A"},"allowed_temperature_rise":{"value":"10","unit":"K"},"isolation_group":"Test secondary"});
    let args = request(&session, &class_target, &class_values);
    session.execute(SET, &args).unwrap();
    assert_eq!(inspect(&mut session, &class_target), class_values);
    assert_eq!(session.project().unwrap().net_classes().get(class).unwrap().revision, 2);
    apply(&mut session, net_classes::UPDATE, json!({"class":class,"name":"Renamed class","rules":{}}));
    assert_eq!(inspect(&mut session, &class_target), class_values);
    let clear = request(&session, &class_target, &json!({}));
    session.apply(SET, &clear).unwrap();
    assert_eq!(inspect(&mut session, &class_target), unset);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(inspect(&mut session, &class_target), class_values);
    assert_eq!(inspect(&mut session, &net_target), values);
}

/// Wrong dimensions, unsupported precision, sign, names, references and stale requests have no
/// effect on the command snapshot or either history stack, even while a redo is available.
#[test]
fn authored_electrical_intent_refuses_invalid_or_stale_edits_without_mutation() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    let (address, _) = named_net(&mut session, "TEST_NET", 0);
    session.apply("project.rename", &json!({"name":"Later name"})).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    let target = json!({"kind":"net","address":address});
    let valid = json!({"working_voltage":{"magnitude":{"value":"-12","unit":"V"},"basis":"dc","reference":{"kind":"earth"}},
        "maximum_continuous_current":{"value":"0","unit":"A"},"allowed_temperature_rise":{"value":"0","unit":"K"}});
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let history = (session.undo.len(), session.redo.len());
    let good = request(&session, &target, &valid);
    let mut stale = good.clone();
    stale["expected_revision"] = json!(revision - 1);
    assert!(matches!(session.apply(SET, &stale), Err(CommandError::Unavailable(_))));
    let mut wrong_version = good.clone();
    wrong_version["version"] = json!(2);
    assert!(matches!(session.apply(SET, &wrong_version), Err(CommandError::InvalidParams(_))));
    let mut missing_revision = good.clone();
    missing_revision.as_object_mut().unwrap().remove("expected_revision");
    assert!(matches!(session.apply(SET, &missing_revision), Err(CommandError::InvalidParams(_))));
    for invalid_intent in [
        json!({"maximum_continuous_current":{"value":"1","unit":"V"}}),
        json!({"maximum_continuous_current":{"value":"1"}}),
        json!({"maximum_continuous_current":{"value":"-0.000000000000000001","unit":"A"}}),
        json!({"allowed_temperature_rise":{"value":"-0.000000000000000001","unit":"K"}}),
        json!({"allowed_temperature_rise":{"value":"0.0000000000000000001","unit":"K"}}),
        json!({"working_voltage":{"magnitude":{"value":"-1","unit":"V"},"basis":"peak_ac","reference":{"kind":"earth"}}}),
        json!({"working_voltage":{"magnitude":{"value":"1","unit":"V"},"basis":"dc"}}),
        json!({"working_voltage":{"magnitude":{"value":"1","unit":"V"},"basis":"dc","reference":{"kind":"net","address":address}}}),
        json!({"working_voltage":{"magnitude":{"value":"1","unit":"V"},"basis":"dc","reference":{"kind":"net","address":NetAddress { net: Uuid::from_u128(1), ..address }}}}),
        json!({"isolation_group":" "}),
        json!({"isolation_group":"\nprimary"}),
        json!({"derived_width":0}),
    ] {
        let bad = request(&session, &target, &invalid_intent);
        assert!(matches!(session.apply(SET, &bad), Err(CommandError::InvalidParams(_))), "{bad}");
    }
    let mut foreign = good.clone();
    foreign["target"]["address"]["project"] = json!(Uuid::nil());
    assert!(matches!(session.apply(SET, &foreign), Err(CommandError::InvalidParams(_))));
    let mut preview = good.clone();
    preview["dry_run"] = json!(true);
    preview["intent"]["allowed_temperature_rise"]["unit"] = json!("A");
    assert!(matches!(session.apply(SET, &preview), Err(CommandError::InvalidParams(_))));
    assert_eq!(session.revision(), revision);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    session.apply(SET, &good).unwrap();
    assert_eq!(inspect(&mut session, &target)["working_voltage"], valid["working_voltage"]);
}

/// Display renames do not retarget references. Deleting the reference retains its UUID as
/// authored history; a different target cannot use that retired net for a new reference.
#[test]
fn authored_electrical_intent_reference_survives_rename_and_retirement() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    let (address, _) = named_net(&mut session, "TEST_SOURCE", 0);
    let (reference, label) = named_net(&mut session, "TEST_REF", 25_400_000);
    let (other, _) = named_net(&mut session, "TEST_OTHER", 50_800_000);
    let target = json!({"kind":"net","address":address});
    let reference_target = json!({"kind":"net","address":reference});
    let voltage = json!({"magnitude":{"value":"3.3","unit":"V"},"basis":"dc","reference":{"kind":"net","address":reference}});
    let requested = json!({"working_voltage":voltage});
    let args = request(&session, &target, &requested);
    session.apply(SET, &args).unwrap();
    apply(&mut session, net_labels::SET, json!({"id":label,"name":"TEST_REF_RENAMED","style":"local","x_nm":25_400_000,"y_nm":0}));
    assert_eq!(inspect(&mut session, &target)["working_voltage"], voltage);
    apply(&mut session, net_labels::DELETE, json!({"id":label}));
    assert!(matches!(session.project().unwrap().net_registry.record(reference.net).unwrap().state, State::Retired { .. }));
    assert_eq!(inspect(&mut session, &target)["working_voltage"], voltage);
    let args = request(&session, &json!({"kind":"net","address":other}), &requested);
    assert!(matches!(session.apply(SET, &args), Err(CommandError::InvalidParams(_))));
    let args = request(&session, &reference_target, &json!({"isolation_group":"Retired"}));
    assert!(matches!(session.apply(SET, &args), Err(CommandError::Unavailable(_))));

    let mut keep = requested;
    keep["maximum_continuous_current"] = json!({"value":"4","unit":"A"});
    let args = request(&session, &target, &keep);
    session.apply(SET, &args).unwrap();
    assert_eq!(inspect(&mut session, &target)["working_voltage"], voltage);
}

/// The same net references survive first-board adoption. Only their owner qualification changes;
/// a request with the old scope and a fresh revision is still refused.
#[test]
fn authored_electrical_intent_reference_survives_first_board_adoption() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    let (source, _) = named_net(&mut session, "TEST_SOURCE", 0);
    let (reference, _) = named_net(&mut session, "TEST_REF", 25_400_000);
    let target = json!({"kind":"net","address":source});
    let requested = json!({"working_voltage":{
        "magnitude":{"value":"5","unit":"V"},"basis":"dc","reference":{"kind":"net","address":reference}
    }});
    let args = request(&session, &target, &requested);
    session.apply(SET, &args).unwrap();
    apply(&mut session, crate::board::identity::CREATE_BOARD, json!({}));
    let scope = session.project().unwrap().net_scope();
    assert_ne!(scope, source.scope);
    let moved = json!({"kind":"net","address":NetAddress { scope, ..source }});
    let values = inspect(&mut session, &moved);
    assert_eq!(values["working_voltage"]["reference"]["address"], json!(NetAddress { scope, ..reference }));
    let stale_scope = request(&session, &target, &requested);
    assert!(matches!(session.apply(SET, &stale_scope), Err(CommandError::InvalidParams(_))));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(inspect(&mut session, &target)["working_voltage"], requested["working_voltage"]);
}
