//! Component-class acceptance through the shared command path and the native codecs.
use super::*;
use crate::{
    Session,
    persistence::{ENTRY_POINT, decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Apply a revision-checked command exactly as GUI and MCP callers do.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    if !matches!(command, "project.new" | "edit.undo" | "edit.redo") {
        params["expected_revision"] = json!(session.revision());
    }
    session.apply(command, &params).unwrap();
}

/// The same request as `apply`, returning the refusal instead of unwrapping it.
fn refuse(session: &mut Session, command: &str, mut params: Value) -> String {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap_err().to_string()
}

/// A project with `count` placed resistors, optionally with a board, and their component IDs.
fn parts(count: usize, board: bool) -> (Session, Vec<String>) {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({"create_board":board}));
    for index in 0..count {
        apply(&mut session, crate::PLACE_RESISTOR, json!({"x_nm":index as i64*25_400_000,"y_nm":0}));
    }
    let components = session.project().unwrap().components().map(|component| component.id()).collect();
    (session, components)
}

/// Create a class and return its UUID from inspection, never from its name alone.
fn create(session: &mut Session, name: &str) -> Uuid {
    apply(session, CREATE, json!({"name":name}));
    let report = session.execute(INSPECT, &json!({})).unwrap();
    let definitions = report["component_classes"]["definitions"].as_array().unwrap();
    serde_json::from_value(definitions.iter().rev().find(|class| class["name"] == name).unwrap()["id"].clone()).unwrap()
}

/// Every membership as (component address, class), the form the done test compares.
fn memberships(project: &Project) -> Vec<(String, Uuid)> {
    project.component_class_bindings().iter().map(|binding| (component_address(binding.component), binding.class)).collect()
}

/// Parse one saved JSON document.
fn document(docs: &crate::persistence::Documents, path: &str) -> Value {
    serde_json::from_slice(&docs[path]).unwrap()
}

/// Replace one saved document with an edited JSON value.
fn edited(docs: &crate::persistence::Documents, path: &str, edit: impl FnOnce(&mut Value)) -> crate::persistence::Documents {
    let mut value = document(docs, path);
    edit(&mut value);
    let mut docs = docs.clone();
    docs.insert(path.into(), serde_json::to_vec_pretty(&value).unwrap());
    docs
}

/// Save and reopen through folders and the packed archive, returning the folder documents.
fn round_trip(session: &Session) -> crate::persistence::Documents {
    let project = session.snapshot().unwrap();
    let docs = encode_documents(project.clone()).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert_eq!(reopened.component_classes(), project.component_classes());
    assert_eq!(memberships(&reopened), memberships(&project));
    assert_eq!(encode_documents(reopened).unwrap(), docs);
    assert_eq!(decode_packed(&encode_packed(project).unwrap()).unwrap().documents, docs);
    docs
}

/// Create a class, assign three components, save and reload: membership by component ID is
/// unchanged, with the schematic-only and the board-owned instance home.
#[test]
fn three_memberships_survive_save_and_reload_by_component_id() {
    for board in [false, true] {
        let (mut s, components) = parts(3, board);
        // A numbered net puts the registry ahead of the memberships in the instance home.
        let pins: Vec<_> = s.project().unwrap().symbols().iter().take(2).map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"})).collect();
        apply(&mut s, crate::CONNECT_PINS, json!({"from":pins[0],"to":pins[1]}));
        assert_eq!(document(&encode_documents(s.snapshot().unwrap()).unwrap(), ENTRY_POINT)["format"], "pcbcraft-project/23");
        let power = create(&mut s, "Power");
        for component in &components {
            apply(&mut s, SET_CLASSES, json!({"component":component,"classes":[power]}));
        }
        let before = memberships(s.project().unwrap());
        assert_eq!(before, components.iter().map(|component| (component.clone(), power)).collect::<Vec<_>>());
        let docs = round_trip(&s);
        let reopened = decode_documents(&docs).unwrap();
        assert_eq!(memberships(&reopened), before);
        let project = document(&docs, ENTRY_POINT);
        assert_eq!(project["format"], "pcbcraft-project/24");
        assert_eq!(project["component_classes"]["format"], FORMAT);
        assert_eq!(reopened.net_registry, s.project().unwrap().net_registry);
        assert!(reopened.net_registry.has_generated_numbers());
        let home = match s.project().unwrap().instances_document_path() {
            Some(path) => {
                assert!(board);
                let instances = document(&docs, &path);
                assert_eq!(instances["format"], "pcbcraft-instances/5");
                assert!(project.get("schematic_instances").is_none());
                instances
            }
            None => {
                assert!(!board);
                project["schematic_instances"].clone()
            }
        };
        let keys: Vec<_> = components.iter().map(|component| component_key(component).unwrap()).collect();
        assert_eq!(home["component_class_bindings"], json!(keys.iter().map(|key| json!({"component":key,"class":power})).collect::<Vec<_>>()));
        let report = s.execute(INSPECT_CLASSES, &json!({"component":components[1]})).unwrap();
        assert_eq!(report["component_memberships"], json!([{"component":components[1],"classes":[power]}]));
    }
}

/// Projects without component classes keep their format and bytes; a catalog without members
/// writes project/24 but leaves the board instance header unchanged.
#[test]
fn untouched_projects_keep_their_bytes_and_headers() {
    for board in [false, true] {
        let (mut s, _) = parts(2, board);
        let untouched = encode_documents(s.snapshot().unwrap()).unwrap();
        assert_ne!(document(&untouched, ENTRY_POINT)["format"], "pcbcraft-project/24");
        assert!(document(&untouched, ENTRY_POINT).get("component_classes").is_none());
        let class = create(&mut s, "Connectors");
        let docs = round_trip(&s);
        assert_eq!(document(&docs, ENTRY_POINT)["format"], "pcbcraft-project/24");
        if let Some(path) = s.project().unwrap().instances_document_path() {
            assert_eq!(document(&docs, &path)["format"], document(&untouched, &path)["format"]);
            assert!(document(&docs, &path).get("component_class_bindings").is_none());
        } else {
            assert!(document(&docs, ENTRY_POINT)["schematic_instances"].get("component_class_bindings").is_none());
        }
        apply(&mut s, REMOVE, json!({"class":class}));
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), untouched);
        apply(&mut s, "edit.undo", json!({}));
        apply(&mut s, "edit.undo", json!({}));
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), untouched);
    }
}

/// Older headers carrying component-class records, explicit empties, foreign memberships and unknown
/// fields are refused rather than silently dropped.
#[test]
fn malformed_and_downgraded_documents_are_refused() {
    for board in [false, true] {
        let (mut s, components) = parts(2, board);
        let class = create(&mut s, "Power");
        apply(&mut s, SET_CLASSES, json!({"component":components[0],"classes":[class]}));
        let docs = round_trip(&s);
        let path = s.project().unwrap().instances_document_path();
        let home = path.clone().unwrap_or_else(|| ENTRY_POINT.into());
        fn field(value: &mut Value, board: bool) -> &mut Value {
            if board { &mut value["component_class_bindings"] } else { &mut value["schematic_instances"]["component_class_bindings"] }
        }
        let bindings = |value: &mut Value, list: Value| *field(value, board) = list;
        let mut cases = vec![
            edited(&docs, ENTRY_POINT, |project| project["format"] = json!("pcbcraft-project/23")),
            edited(&docs, ENTRY_POINT, |project| project["component_classes"]["definitions"] = json!([])),
            edited(&docs, ENTRY_POINT, |project| project["component_classes"] = Value::Null),
            edited(&docs, ENTRY_POINT, |project| project["component_classes"]["format"] = json!("pcbcraft-component-classes/2")),
            edited(&docs, ENTRY_POINT, |project| project["component_classes"]["definitions"][0]["rules"] = json!({})),
            edited(&docs, &home, |value| bindings(value, json!([]))),
            edited(&docs, &home, |value| bindings(value, Value::Null)),
            edited(&docs, &home, |value| bindings(value, json!([{"component":999,"class":class}]))),
            edited(&docs, &home, |value| bindings(value, json!([{"component":component_key(&components[0]),"class":Uuid::from_u128(7)}]))),
            edited(&docs, &home, |value| bindings(value, json!([{"component":component_key(&components[0]),"class":class,"origin":1}]))),
            edited(&docs, &home, |value| {
                bindings(
                    value,
                    json!([{"component":component_key(&components[1]),"class":class},{"component":component_key(&components[0]),"class":class}]),
                )
            }),
        ];
        if let Some(path) = &path {
            cases.push(edited(&docs, path, |instances| instances["format"] = json!("pcbcraft-instances/4")));
        }
        for case in cases {
            assert!(decode_documents(&case).is_err());
        }
    }
}

/// Catalog edits are revisioned and atomic; removing a class with members is refused and
/// names them; no-op requests leave history and revision untouched; undo restores both.
#[test]
fn catalog_and_membership_edits_are_atomic_and_undoable() {
    let (mut s, components) = parts(3, false);
    let power = create(&mut s, "Power");
    let logic = create(&mut s, "Logic");
    apply(&mut s, UPDATE, json!({"class":power,"name":"Supply"}));
    let supply = s.project().unwrap().component_classes().get(power).unwrap().clone();
    assert_eq!((supply.name.as_str(), supply.revision), ("Supply", 2));
    let revision = s.revision();
    apply(&mut s, UPDATE, json!({"class":power,"name":"Supply"}));
    apply(&mut s, SET_CLASSES, json!({"component":components[0],"classes":[]}));
    assert_eq!(s.revision(), revision);

    refuse(&mut s, SET_CLASSES, json!({"component":components[0],"classes":[logic,power,power]}));
    assert_eq!(s.revision(), revision);
    apply(&mut s, SET_CLASSES, json!({"component":components[0],"classes":[logic,power]}));
    apply(&mut s, SET_CLASSES, json!({"component":components[2],"classes":[power]}));
    let assigned = s.snapshot().unwrap();
    let revision = s.revision();
    apply(&mut s, SET_CLASSES, json!({"component":components[0],"classes":[power,logic]}));
    assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
    let mut expected = vec![power, logic];
    expected.sort();
    assert_eq!(s.project().unwrap().classes_of_component(component_key(&components[0]).unwrap()).collect::<Vec<_>>(), expected);

    let refusal = refuse(&mut s, REMOVE, json!({"class":power}));
    assert!(refusal.contains(&components[0]) && refusal.contains(&components[2]) && !refusal.contains(&components[1]), "{refusal}");
    for (command, params) in [
        (SET_CLASSES, json!({"component":"component-999","classes":[power]})),
        (SET_CLASSES, json!({"component":components[1],"classes":[Uuid::from_u128(5)]})),
        (SET_CLASSES, json!({"component":"R1","classes":[power]})),
        (UPDATE, json!({"class":power,"name":" padded"})),
        (UPDATE, json!({"class":Uuid::from_u128(5),"name":"Other"})),
        (REMOVE, json!({"class":Uuid::from_u128(5)})),
    ] {
        refuse(&mut s, command, params);
        assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
    }
    let stale = json!({"component":components[1],"classes":[power],"expected_revision":s.revision() - 1});
    assert!(s.apply(SET_CLASSES, &stale).is_err());
    assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));

    apply(&mut s, SET_CLASSES, json!({"component":components[2],"classes":[]}));
    apply(&mut s, SET_CLASSES, json!({"component":components[0],"classes":[logic]}));
    apply(&mut s, REMOVE, json!({"class":power}));
    assert!(s.project().unwrap().component_classes().get(power).is_none());
    apply(&mut s, "edit.undo", json!({}));
    apply(&mut s, "edit.undo", json!({}));
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(memberships(s.project().unwrap()), memberships(&assigned));
    apply(&mut s, "edit.redo", json!({}));
    assert_eq!(s.project().unwrap().classes_of_component(component_key(&components[2]).unwrap()).count(), 0);
    let report = s.execute(INSPECT_CLASSES, &json!({})).unwrap();
    assert_eq!(report["component_memberships"].as_array().unwrap().len(), 1);
}

/// Deleting a component removes its memberships in the same history step; undo restores them.
#[test]
fn deleting_a_component_takes_its_memberships_with_it() {
    let (mut s, components) = parts(2, false);
    let class = create(&mut s, "Power");
    for component in &components {
        apply(&mut s, SET_CLASSES, json!({"component":component,"classes":[class]}));
    }
    let assigned = memberships(s.project().unwrap());
    let key = component_key(&components[0]).unwrap();
    let symbol = s.project().unwrap().symbols().iter().find(|symbol| symbol.component == key).unwrap().id();
    apply(&mut s, crate::DELETE_SYMBOL, json!({"symbol":symbol}));
    assert_eq!(memberships(s.project().unwrap()), vec![(components[1].clone(), class)]);
    round_trip(&s);
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(memberships(s.project().unwrap()), assigned);
}

/// The single accessor pair is the only conversion between addresses and stored keys.
#[test]
fn component_keys_round_trip_through_addresses() {
    assert_eq!(component_key("component-42"), Some(42));
    assert_eq!(component_address(42), "component-42");
    for invalid in ["R1", "component-", "component-x", "42"] {
        assert_eq!(component_key(invalid), None);
    }
}
