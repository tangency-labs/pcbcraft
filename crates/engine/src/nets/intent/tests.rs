use super::{INSPECT, SET};
use crate::persistence::{Documents, ENTRY_POINT, decode_documents, decode_packed, encode_documents, encode_packed};
use crate::{
    CommandError, PLACE_SYMBOL, SET_SYMBOL_FIELDS, Session, net_classes, net_labels,
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
    stale["target"] = Value::Null;
    assert_eq!(session.apply(SET, &stale), Err(CommandError::StaleRevision { current: revision }));
    assert_eq!(session.execute(SET, &stale), Err(CommandError::StaleRevision { current: revision }));
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

/// Read a production-encoded split document without manufacturing a fixture by hand.
fn document(docs: &Documents, path: &str) -> Value {
    serde_json::from_slice(&docs[path]).unwrap()
}

/// Mutate exactly one encoded document to exercise refusal of mismatched or invalid payloads.
fn altered(docs: &Documents, path: &str, change: impl FnOnce(&mut Value)) -> Documents {
    let mut value = document(docs, path);
    change(&mut value);
    let mut result = docs.clone();
    result.insert(path.into(), serde_json::to_vec(&value).unwrap());
    result
}

/// Check the named compatibility failure, not merely that decoding failed.
fn refused(docs: &Documents, reason: &str) {
    let error = decode_documents(docs).unwrap_err();
    assert!(error.contains(reason), "expected {reason:?}, got {error:?}");
}

/// Both storage homes preserve exact values, use lazy headers independently, and refuse older
/// header combinations instead of treating new authored fields as unset. Clearing net-only
/// values restores exact pre-edit native bytes; Undo restores exact intent bytes.
#[test]
fn authored_electrical_intent_native_lazy_round_trip_and_refusals() {
    for board in [false, true] {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":board})).unwrap();
        let (source, _) = named_net(&mut session, "TEST_SOURCE", 0);
        let (reference, _) = named_net(&mut session, "TEST_REFERENCE", 25_400_000);
        apply(&mut session, net_classes::CREATE, json!({"name":"Supply class","rules":{}}));
        let class = session.project().unwrap().net_classes().definitions[0].id;
        let class_target = json!({"kind":"net_class","project":source.project,"class":class});
        let net_target = json!({"kind":"net","address":source});
        let path = session.project().unwrap().instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
        let old = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&old).unwrap()).unwrap(), old);
        assert_eq!(document(&old, ENTRY_POINT)["format"], "pcbcraft-project/23");
        refused(&altered(&old, ENTRY_POINT, |v| v["format"] = json!("pcbcraft-project/49")), "may not be empty");
        let values = json!({"maximum_continuous_current":{"value":"0","unit":"A"},"working_voltage":{
            "magnitude":{"value":"0","unit":"V"},"basis":"dc","reference":{"kind":"net","address":reference}
        }});
        let args = request(&session, &net_target, &values);
        session.apply(SET, &args).unwrap();
        let net_only = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(document(&net_only, ENTRY_POINT)["format"], "pcbcraft-project/49");
        assert_eq!(document(&net_only, ENTRY_POINT)["net_classes"]["format"], "pcbcraft-net-classes/1");
        let old_header = altered(&net_only, ENTRY_POINT, |v| v["format"] = json!("pcbcraft-project/23"));
        refused(&old_header, if board { "unsupported instances format" } else { "requires project format 49" });
        if board {
            assert_eq!(document(&net_only, &path)["format"], "pcbcraft-instances/10");
            refused(&altered(&net_only, &path, |v| v["format"] = json!("pcbcraft-instances/4")), "unsupported instances format");
            refused(&altered(&old, &path, |v| v["format"] = json!("pcbcraft-instances/10")), "unsupported instances format");
        }
        assert_eq!(encode_documents(decode_documents(&net_only).unwrap()).unwrap(), net_only);
        let clear = request(&session, &net_target, &json!({}));
        session.apply(SET, &clear).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), old);
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), net_only);

        let class_values = json!({"allowed_temperature_rise":{"value":"12.5","unit":"K"},"isolation_group":"Test primary",
            "working_voltage":values["working_voltage"]});
        let args = request(&session, &class_target, &class_values);
        session.apply(SET, &args).unwrap();
        let both = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(document(&both, ENTRY_POINT)["net_classes"]["format"], "pcbcraft-net-classes/4");
        for old_catalog in ["pcbcraft-net-classes/1", "pcbcraft-net-classes/2"] {
            refused(&altered(&both, ENTRY_POINT, |v| v["net_classes"]["format"] = json!(old_catalog)), "Unsupported net-class format");
        }
        refused(
            &altered(&both, ENTRY_POINT, |v| {
                v["net_classes"]["definitions"][0]["authored_intent"]["voltage"]["reference"]["net"] = json!(Uuid::nil());
            }),
            "distinct reference",
        );
        for (replacement, reason) in [(json!({}), "nonempty"), (Value::Null, "invalid type"), (json!({"current_amperes":"-1"}), "canonical")] {
            let malformed = altered(&both, &path, |v| {
                let home = if board { v } else { &mut v["schematic_instances"] };
                let net = home["net_registry"]["records"].as_array_mut().unwrap().iter_mut().find(|v| v["id"] == json!(source.net)).unwrap();
                net["authored_intent"] = replacement;
            });
            refused(&malformed, reason);
        }
        let archive = decode_packed(&encode_packed(session.snapshot().unwrap()).unwrap()).unwrap();
        assert_eq!(archive.documents, both);
        assert_eq!(encode_documents(decode_documents(&archive.documents).unwrap()).unwrap(), both);
        session.open_snapshot(decode_documents(&archive.documents).unwrap());
        assert_eq!(inspect(&mut session, &net_target)["maximum_continuous_current"], values["maximum_continuous_current"]);
        assert_eq!(inspect(&mut session, &class_target)["working_voltage"], class_values["working_voltage"]);
        let clear = request(&session, &net_target, &json!({}));
        session.apply(SET, &clear).unwrap();
        let class_only = encode_documents(session.snapshot().unwrap()).unwrap();
        if board {
            assert_eq!(class_only[&path], old[&path]);
        }
        assert_eq!(document(&class_only, ENTRY_POINT)["format"], "pcbcraft-project/49");
        assert_eq!(encode_documents(decode_documents(&class_only).unwrap()).unwrap(), class_only);
        let clear = request(&session, &class_target, &json!({}));
        session.apply(SET, &clear).unwrap();
        let cleared = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(document(&cleared, ENTRY_POINT)["format"], "pcbcraft-project/23");
        assert_eq!(document(&cleared, ENTRY_POINT)["net_classes"]["format"], "pcbcraft-net-classes/1");
    }
}

/// An existing unnumbered net registry does not acquire numbering as a side effect of intent.
/// Clearing the last value returns it to the exact legacy encoding, not a numbered upgrade.
#[test]
fn authored_electrical_intent_on_legacy_net_does_not_adopt_numbers() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    let (net, _) = named_net(&mut session, "TEST_LEGACY", 0);
    let modern = encode_documents(session.snapshot().unwrap()).unwrap();
    let legacy = altered(&modern, ENTRY_POINT, |v| {
        v["format"] = json!("pcbcraft-project/17");
        v["schematic_instances"]["net_registry"]["records"][0].as_object_mut().unwrap().remove("generated_number");
    });
    session.open_snapshot(decode_documents(&legacy).unwrap());
    let canonical = encode_documents(session.snapshot().unwrap()).unwrap();
    let target = json!({"kind":"net","address":net});
    let auth = request(&session, &target, &json!({"maximum_continuous_current":{"value":"0","unit":"A"}}));
    session.apply(SET, &auth).unwrap();
    assert!(!session.project().unwrap().net_registry.has_generated_numbers());
    let encoded = encode_documents(session.snapshot().unwrap()).unwrap();
    session.open_snapshot(decode_documents(&encoded).unwrap());
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), encoded);
    let clear = request(&session, &target, &json!({}));
    session.apply(SET, &clear).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), canonical);
}

/// The intent envelope composes with profiles, assigned dimensional classes, parameters and
/// both membership representations in both instance homes without changing their payloads.
#[test]
fn authored_electrical_intent_composes_with_profiles_and_memberships() {
    for board in [false, true] {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":board})).unwrap();
        let (net, _) = named_net(&mut session, "TEST_SUPPLY", 0);
        apply(&mut session, crate::PLACE_RESISTOR, json!({"x_nm":50_800_000,"y_nm":0}));
        let part = session.project().unwrap().part_instance_addresses().next().unwrap();
        apply(&mut session, crate::part_groups::CREATE, json!({"name":"Test group"}));
        let group = session.project().unwrap().part_groups().groups[0].id;
        apply(&mut session, crate::part_groups::SET_GROUPS, json!({"part_instance":part,"groups":[group]}));
        apply(&mut session, crate::parameters::SET, json!({"scope":"project","name":"Revision","value":"B"}));
        apply(&mut session, crate::no_connect::SET, json!({"target":{"part":part,"terminal":"2"},"marked":true}));
        apply(&mut session, net_classes::CREATE, json!({"name":"Power","rules":{"clearance_nm":500000}}));
        let class = session.project().unwrap().net_classes().definitions[0].id;
        apply(
            &mut session,
            crate::nets::SET_CLASSES,
            json!({"target":{"kind":"persistent","project":net.project,"scope":net.scope,"net":net.net},"classes":[class]}),
        );
        if board {
            apply(
                &mut session,
                crate::SET_ASSEMBLY_PROFILE,
                json!({"content":{"revision":1,"name":"Test profile","provenance":{"kind":"authored"},"bottom_side":"viewed_from_top"}}),
            );
        }
        let target = json!({"kind":"net","address":net});
        let intent = json!({"isolation_group":"Test group"});
        let expanded = session.snapshot().unwrap();
        let path = expanded.instances_document_path();
        for compact in [false, true] {
            let mut before = (*expanded).clone();
            // Use the existing encoding policy switch; the membership data came from commands.
            before.prefer_compact_part_groups = compact;
            session.open_snapshot(Arc::new(before));
            let old = encode_documents(session.snapshot().unwrap()).unwrap();
            let old_format = if board {
                "pcbcraft-project/36"
            } else if compact {
                "pcbcraft-project/39"
            } else {
                "pcbcraft-project/24"
            };
            assert_eq!(document(&old, ENTRY_POINT)["format"], old_format);
            let args = request(&session, &target, &intent);
            session.apply(SET, &args).unwrap();
            let changed = encode_documents(session.snapshot().unwrap()).unwrap();
            let project = document(&changed, ENTRY_POINT);
            let old_project = document(&old, ENTRY_POINT);
            let membership = if compact { "compact_part_group_memberships" } else { "part_group_memberships" };
            assert_eq!(project["format"], "pcbcraft-project/49");
            for field in ["assembly_profiles", "parameters", "components", "net_classes", "part_groups"] {
                assert_eq!(project[field], old_project[field]);
            }
            if let Some(path) = &path {
                let instances = document(&changed, path);
                assert_eq!(instances["format"], "pcbcraft-instances/10");
                assert_eq!(instances[membership], document(&old, path)[membership]);
            } else {
                assert_eq!(project["schematic_instances"][membership], old_project["schematic_instances"][membership]);
            }
            session.open_snapshot(decode_documents(&changed).unwrap());
            assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), changed);
            let clear = request(&session, &target, &json!({}));
            session.apply(SET, &clear).unwrap();
            assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), old);
        }
    }
}

/// Intent composes with stored differential pairs. Net intent moves the pair envelope to
/// project/49 and instances/10 without changing the pair list; class-only intent keeps
/// instances/8; clearing the last value restores the exact pair-envelope bytes.
#[test]
fn authored_electrical_intent_composes_with_differential_pairs() {
    for board in [false, true] {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":board})).unwrap();
        let (positive, _) = named_net(&mut session, "TEST_P", 0);
        let (negative, _) = named_net(&mut session, "TEST_N", 25_400_000);
        let pair = crate::nets::Pair {
            id: Uuid::from_u128(9000),
            positive: positive.net,
            negative: negative.net,
            name: "TEST".into(),
            state: crate::nets::PairState::Active {},
        };
        let mut project = session.project().unwrap().clone();
        Arc::make_mut(&mut project.net_registry).pairs = Arc::new(vec![Arc::new(pair)]);
        session.change(project).unwrap();
        let path = session.project().unwrap().instances_document_path();
        let home = path.clone().unwrap_or_else(|| ENTRY_POINT.into());
        let pairs = |docs: &Documents| {
            let value = document(docs, &home);
            if path.is_some() { value["net_registry"]["pairs"].clone() } else { value["schematic_instances"]["net_registry"]["pairs"].clone() }
        };
        let old = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(document(&old, ENTRY_POINT)["format"], "pcbcraft-project/45");
        assert!(pairs(&old).as_array().is_some_and(|items| items.len() == 1));

        let target = json!({"kind":"net","address":positive});
        let args = request(&session, &target, &json!({"isolation_group":"Test pair"}));
        session.apply(SET, &args).unwrap();
        let changed = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(document(&changed, ENTRY_POINT)["format"], "pcbcraft-project/49");
        assert_eq!(pairs(&changed), pairs(&old));
        if let Some(path) = &path {
            assert_eq!(document(&changed, path)["format"], "pcbcraft-instances/10");
            refused(&altered(&changed, path, |v| v["format"] = json!("pcbcraft-instances/8")), "unsupported instances format");
        }
        let pair_header = altered(&changed, ENTRY_POINT, |v| v["format"] = json!("pcbcraft-project/45"));
        refused(&pair_header, if board { "unsupported instances format" } else { "requires project format 49" });
        assert_eq!(encode_documents(decode_documents(&changed).unwrap()).unwrap(), changed);
        let clear = request(&session, &target, &json!({}));
        session.apply(SET, &clear).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), old);

        apply(&mut session, net_classes::CREATE, json!({"name":"Pair class","rules":{}}));
        let class = session.project().unwrap().net_classes().definitions[0].id;
        let class_target = json!({"kind":"net_class","project":positive.project,"class":class});
        let args = request(&session, &class_target, &json!({"isolation_group":"Test pair"}));
        session.apply(SET, &args).unwrap();
        let class_only = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(document(&class_only, ENTRY_POINT)["format"], "pcbcraft-project/49");
        assert_eq!(pairs(&class_only), pairs(&old));
        if let Some(path) = &path {
            assert_eq!(class_only[path], old[path]);
            assert_eq!(document(&class_only, path)["format"], "pcbcraft-instances/8");
        }
        assert_eq!(encode_documents(decode_documents(&class_only).unwrap()).unwrap(), class_only);
    }
}

/// Authored intent and an active designator lock compose under project/54 only while both
/// are present. Save and reopen preserve both exactly; each single-feature header refuses the
/// composed content, and clearing either feature returns to that feature's own header.
#[test]
fn authored_electrical_intent_composes_with_designator_locks() {
    for board in [false, true] {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":board})).unwrap();
        for x_nm in [0, 25_400_000] {
            apply(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":x_nm,"y_nm":0}));
        }
        let unlocked = encode_documents(session.snapshot().unwrap()).unwrap();
        let (address, _) = named_net(&mut session, "TEST_LOCKED", 50_800_000);
        let lock = |session: &mut Session, locked: bool| {
            apply(session, SET_SYMBOL_FIELDS, json!({"symbol":"symbol-2","reference":"R1","value":"R","designator_locked":locked}));
        };
        lock(&mut session, true);
        let locks_only = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_ne!(locks_only, unlocked);
        assert_eq!(document(&locks_only, ENTRY_POINT)["format"], "pcbcraft-project/51");

        let target = json!({"kind":"net","address":address});
        let args = request(&session, &target, &json!({"isolation_group":"Locked primary"}));
        session.apply(SET, &args).unwrap();
        let both = encode_documents(session.snapshot().unwrap()).unwrap();
        let entry = document(&both, ENTRY_POINT);
        assert_eq!(entry["format"], "pcbcraft-project/54");
        assert_eq!(entry["components"][0]["designator_locked"], true);
        if let Some(path) = session.project().unwrap().instances_document_path() {
            assert_eq!(document(&both, &path)["format"], "pcbcraft-instances/10");
        }
        refused(&altered(&both, ENTRY_POINT, |v| v["format"] = json!("pcbcraft-project/49")), "Designator locks require project format 51");
        let lock_header = altered(&both, ENTRY_POINT, |v| v["format"] = json!("pcbcraft-project/51"));
        refused(&lock_header, if board { "unsupported instances format" } else { "requires project format 49" });

        let reopened = decode_documents(&both).unwrap();
        assert_eq!(encode_documents(reopened.clone()).unwrap(), both);
        assert_eq!(encode_documents(decode_packed(&encode_packed(reopened.clone()).unwrap()).unwrap().project).unwrap(), both);
        let mut reader = Session::default();
        reader.open_snapshot(reopened);
        assert_eq!(inspect(&mut reader, &target)["isolation_group"], "Locked primary");
        assert!(reader.project().unwrap().components().next().unwrap().designator_locked());

        let clear = request(&session, &target, &json!({}));
        session.apply(SET, &clear).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), locks_only);
        let args = request(&session, &target, &json!({"isolation_group":"Locked primary"}));
        session.apply(SET, &args).unwrap();
        lock(&mut session, false);
        let intent_only = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(document(&intent_only, ENTRY_POINT)["format"], "pcbcraft-project/49");
        let intent_project = decode_documents(&intent_only).unwrap();
        assert_eq!(encode_documents(intent_project.clone()).unwrap(), intent_only);
        let mut reader = Session::default();
        reader.open_snapshot(intent_project);
        assert_eq!(inspect(&mut reader, &target)["isolation_group"], "Locked primary");
        assert!(reader.project().unwrap().components().all(|part| !part.designator_locked()));
    }
}

/// A schematic-only project whose one catalog class holds the given authored intent, its target,
/// and the saved documents with the class intent at `net_classes.definitions[0].authored_intent`.
fn class_with_intent(intent: &Value) -> (Session, Value, Documents) {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    apply(&mut session, net_classes::CREATE, json!({"name":"Review class","rules":{}}));
    let project = session.project().unwrap();
    let target = json!({"kind":"net_class","project":project.id(),"class":project.net_classes().definitions[0].id});
    let args = request(&session, &target, intent);
    session.apply(SET, &args).unwrap();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    (session, target, saved)
}

/// Replace one field of the saved class intent, leaving every other document byte unchanged.
fn saved_intent(docs: &Documents, change: impl FnOnce(&mut Value)) -> Documents {
    altered(docs, ENTRY_POINT, |v| change(&mut v["net_classes"]["definitions"][0]["authored_intent"]))
}

/// Earth carries no fields: a saved Earth reference with a leftover `net` is refused on open
/// rather than read as Earth and silently dropped by the next save.
#[test]
fn saved_earth_reference_refuses_extra_fields() {
    let earth = json!({"working_voltage":{"magnitude":{"value":"5","unit":"V"},"basis":"dc","reference":{"kind":"earth"}}});
    let (_, _, saved) = class_with_intent(&earth);
    assert_eq!(document(&saved, ENTRY_POINT)["net_classes"]["definitions"][0]["authored_intent"]["voltage"]["reference"], json!({"kind":"earth"}));
    for net in [json!(""), json!(Uuid::from_u128(7))] {
        refused(&saved_intent(&saved, |intent| intent["voltage"]["reference"]["net"] = net), "unknown field `net`");
    }
}

/// SET refuses an Earth reference that still carries a net address, without changing the
/// stored reference, revision or history; a plain Earth reference stays valid.
#[test]
fn set_earth_reference_refuses_leftover_address() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    let (address, _) = named_net(&mut session, "TEST_SOURCE", 0);
    let (reference, _) = named_net(&mut session, "TEST_REFERENCE", 25_400_000);
    let target = json!({"kind":"net","address":address});
    let voltage = |reference: Value| json!({"working_voltage":{"magnitude":{"value":"5","unit":"V"},"basis":"dc","reference":reference}});
    let named = voltage(json!({"kind":"net","address":reference}));
    let args = request(&session, &target, &named);
    session.apply(SET, &args).unwrap();
    let before = session.snapshot().unwrap();
    let history = (session.revision(), session.undo.len(), session.redo.len());

    let leftover = request(&session, &target, &voltage(json!({"kind":"earth","address":reference})));
    assert!(matches!(session.apply(SET, &leftover), Err(CommandError::InvalidParams(_))));
    assert!(matches!(session.execute(SET, &leftover), Err(CommandError::InvalidParams(_))));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!((session.revision(), session.undo.len(), session.redo.len()), history);
    assert_eq!(inspect(&mut session, &target)["working_voltage"], named["working_voltage"]);

    let earth = voltage(json!({"kind":"earth"}));
    let args = request(&session, &target, &earth);
    session.apply(SET, &args).unwrap();
    assert_eq!(inspect(&mut session, &target)["working_voltage"], earth["working_voltage"]);
}

/// Command intent records are objects: positional arrays for the intent, a voltage or a quantity
/// are refused without clearing authored values, while `{}` and null properties still clear.
#[test]
fn set_intent_refuses_positional_arrays() {
    let current = json!({"maximum_continuous_current":{"value":"3","unit":"A"}});
    let (mut session, target, _) = class_with_intent(&current);
    let before = session.snapshot().unwrap();
    let history = (session.revision(), session.undo.len(), session.redo.len());
    for positional in [
        json!([null, null, null, null]),
        json!({"maximum_continuous_current":["3","A"]}),
        json!({"allowed_temperature_rise":["1","K"]}),
        json!({"working_voltage":[{"value":"5","unit":"V"},"dc",{"kind":"earth"}]}),
        json!({"working_voltage":{"magnitude":["5","V"],"basis":"dc","reference":{"kind":"earth"}}}),
    ] {
        let args = request(&session, &target, &positional);
        assert!(matches!(session.apply(SET, &args), Err(CommandError::InvalidParams(_))), "{positional}");
        assert!(matches!(session.execute(SET, &args), Err(CommandError::InvalidParams(_))), "{positional}");
    }
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!((session.revision(), session.undo.len(), session.redo.len()), history);
    assert_eq!(inspect(&mut session, &target)["maximum_continuous_current"], current["maximum_continuous_current"]);

    let nulls = json!({"working_voltage":null,"maximum_continuous_current":null,"allowed_temperature_rise":null,"isolation_group":null});
    let args = request(&session, &target, &nulls);
    session.apply(SET, &args).unwrap();
    assert_eq!(inspect(&mut session, &target), nulls);
    session.apply("edit.undo", &Value::Null).unwrap();
    let args = request(&session, &target, &json!({}));
    session.apply(SET, &args).unwrap();
    assert_eq!(inspect(&mut session, &target), nulls);
}

/// Saved intent and its voltage use named fields only: positional arrays are refused on open
/// rather than rewritten as objects on save. Duplicate fields stay refused, and sparse objects
/// still round-trip exactly.
#[test]
fn saved_intent_refuses_positional_records() {
    let values = json!({"working_voltage":{"magnitude":{"value":"5","unit":"V"},"basis":"dc","reference":{"kind":"earth"}},
        "isolation_group":"Review group"});
    let (_, _, saved) = class_with_intent(&values);
    let stored = document(&saved, ENTRY_POINT)["net_classes"]["definitions"][0]["authored_intent"].clone();
    assert_eq!(stored, json!({"voltage":{"magnitude_volts":"5","basis":"dc","reference":{"kind":"earth"}},"isolation_group":"Review group"}));
    refused(&saved_intent(&saved, |intent| intent["voltage"] = json!(["5", "dc", {"kind":"earth"}])), "invalid type: sequence");
    refused(&saved_intent(&saved, |intent| *intent = json!([null, null, null, "Review group"])), "invalid type: sequence");

    let entry = String::from_utf8(saved[ENTRY_POINT].clone()).unwrap();
    let key = "\"isolation_group\"";
    assert_eq!(entry.matches(key).count(), 1);
    let mut duplicate = saved.clone();
    duplicate.insert(ENTRY_POINT.into(), entry.replacen(key, &format!("{key}:\"Other group\",{key}"), 1).into_bytes());
    refused(&duplicate, "duplicate field `isolation_group`");
}
