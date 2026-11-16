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

/// The same request as `apply` at the current revision, which must be refused for `reason`:
/// the message has to contain it, so a refusal for some other cause fails the test.
#[track_caller]
fn refuse(session: &mut Session, command: &str, mut params: Value, reason: &str) -> String {
    params["expected_revision"] = json!(session.revision());
    let refusal = session.apply(command, &params).unwrap_err().to_string();
    assert!(refusal.contains(reason), "{command} {params}: expected {reason:?}, got {refusal:?}");
    refusal
}

/// Decoding must fail for `reason`; any other failure or a success fails the test.
#[track_caller]
fn refused_on_read(documents: &crate::persistence::Documents, reason: &str) {
    let refusal = decode_documents(documents).err().unwrap_or_else(|| panic!("expected {reason:?}, the documents decoded"));
    assert!(refusal.contains(reason), "expected {reason:?}, got {refusal:?}");
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
        /// The membership list in this project's instance home.
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
        let mut reasons = vec![
            "requires project format 24",
            "Empty component-class catalogs must be omitted",
            "invalid type: null, expected struct Catalog",
            "Unsupported component-class format",
            "unknown field `rules`",
            "must be omitted or nonempty",
            "invalid type: null, expected a sequence",
            "must name existing components and classes",
            "must name existing components and classes",
            "unknown field `origin`",
            "sorted by component and class",
        ];
        if let Some(path) = &path {
            cases.push(edited(&docs, path, |instances| instances["format"] = json!("pcbcraft-instances/4")));
            reasons.push("unsupported instances format");
        }
        assert_eq!(cases.len(), reasons.len());
        for (case, reason) in cases.iter().zip(reasons) {
            refused_on_read(case, reason);
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

    refuse(&mut s, SET_CLASSES, json!({"component":components[0],"classes":[logic,power,power]}), "unique non-nil class UUIDs");
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

    let refusal = refuse(&mut s, REMOVE, json!({"class":power}), "still has members");
    assert!(refusal.contains(&components[0]) && refusal.contains(&components[2]) && !refusal.contains(&components[1]), "{refusal}");
    for (command, params, reason) in [
        (SET_CLASSES, json!({"component":"component-999","classes":[power]}), "Unknown component."),
        (SET_CLASSES, json!({"component":components[1],"classes":[Uuid::from_u128(5)]}), "Unknown component class."),
        (SET_CLASSES, json!({"component":"R1","classes":[power]}), "Expected a component ID"),
        (UPDATE, json!({"class":power,"name":" padded"}), "Class name must contain"),
        (UPDATE, json!({"class":Uuid::from_u128(5),"name":"Other"}), "Unknown component class."),
        (REMOVE, json!({"class":Uuid::from_u128(5)}), "Unknown component class."),
    ] {
        refuse(&mut s, command, params, reason);
        assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
    }
    let stale = json!({"component":components[1],"classes":[power],"expected_revision":s.revision() - 1});
    let refusal = s.apply(SET_CLASSES, &stale).unwrap_err().to_string();
    assert!(refusal.contains("project changed"), "{refusal}");
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

/// Names are accepted up to 200 trimmed characters; the first character past the limit, blank,
/// padded and control-containing names are refused on create, update and read.
#[test]
fn class_names_are_bounded_on_each_side_of_the_limit() {
    let (mut s, _) = parts(0, false);
    let longest = "A".repeat(200);
    let class = create(&mut s, &longest);
    assert_eq!(s.project().unwrap().component_classes().get(class).unwrap().name, longest);
    let docs = round_trip(&s);
    let assigned = s.snapshot().unwrap();
    for name in ["A".repeat(201), String::new(), " A".into(), "A ".into(), "A\nB".into()] {
        refuse(&mut s, CREATE, json!({"name":name}), "Class name must contain 1–200");
        refuse(&mut s, UPDATE, json!({"class":class,"name":name}), "Class name must contain 1–200");
        assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
        let saved = edited(&docs, ENTRY_POINT, |project| project["component_classes"]["definitions"][0]["name"] = json!(name));
        refused_on_read(&saved, "Class name must contain 1–200");
    }
    for (params, reason) in
        [(json!({}), "needs `name`"), (json!({"name":null}), "`name` must be a string"), (json!({"name":7}), "`name` must be a string")]
    {
        refuse(&mut s, CREATE, params, reason);
    }
}

/// A catalog holds at most 256 classes: the 256th is created, saved and reopened; the 257th is
/// refused by the command and by the reader.
#[test]
fn catalog_size_is_bounded_on_each_side_of_the_limit() {
    let (mut s, _) = parts(0, false);
    for index in 0..MAX_CLASSES {
        apply(&mut s, CREATE, json!({"name":format!("Class {index}")}));
    }
    assert_eq!(s.project().unwrap().component_classes().definitions.len(), MAX_CLASSES);
    let docs = round_trip(&s);
    let full = s.snapshot().unwrap();
    refuse(&mut s, CREATE, json!({"name":"One more"}), "catalog is full");
    assert!(Arc::ptr_eq(&full, &s.snapshot().unwrap()));
    let overfull = edited(&docs, ENTRY_POINT, |project| {
        project["component_classes"]["definitions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":Uuid::from_u128(u128::MAX),"revision":1,"name":"Extra"}));
    });
    refused_on_read(&overfull, "class limit exceeded");
}

/// Saved revisions run from 1 to `u32::MAX`. The largest is read, and a later rename is then
/// refused as exhausted rather than wrapping; zero, negative and wider values are refused.
#[test]
fn class_revisions_are_bounded_on_each_side_of_the_limit() {
    let (mut s, _) = parts(0, false);
    let class = create(&mut s, "Power");
    let docs = round_trip(&s);
    let revision = |value: Value| edited(&docs, ENTRY_POINT, |project| project["component_classes"]["definitions"][0]["revision"] = value);
    for (bad, reason) in [
        (json!(0), "positive revisions"),
        (json!(-1), "expected u32"),
        (json!(u64::from(u32::MAX) + 1), "expected u32"),
        (json!("1"), "invalid type: string \"1\", expected u32"),
        (Value::Null, "invalid type: null, expected u32"),
    ] {
        refused_on_read(&revision(bad), reason);
    }
    let last = decode_documents(&revision(json!(u32::MAX))).unwrap();
    assert_eq!(last.component_classes().get(class).unwrap().revision, u32::MAX);
    s.open_snapshot(last);
    refuse(&mut s, UPDATE, json!({"class":class,"name":"Supply"}), "revision exhausted");
    assert_eq!(s.project().unwrap().component_classes().get(class).unwrap().name, "Power");
}

/// Saved records with missing, null, wrong-typed or out-of-range fields, missing required
/// records, duplicate identities and unknown fields are refused by the reader.
#[test]
fn saved_record_edges_are_refused() {
    for board in [false, true] {
        let (mut s, components) = parts(2, board);
        let power = create(&mut s, "Power");
        create(&mut s, "Logic");
        apply(&mut s, SET_CLASSES, json!({"component":components[0],"classes":[power]}));
        let docs = round_trip(&s);
        let path = s.project().unwrap().instances_document_path();
        let home = path.clone().unwrap_or_else(|| ENTRY_POINT.into());
        let key = component_key(&components[0]).unwrap();
        let binding = |record: Value| {
            edited(&docs, &home, |value| {
                let list = if board { &mut value["component_class_bindings"] } else { &mut value["schematic_instances"]["component_class_bindings"] };
                *list = json!([record]);
            })
        };
        let mut cases = vec![
            binding(json!({"component":-1,"class":power})),
            binding(json!({"component":components[0],"class":power})),
            {
                // serde_json values cannot hold the first integer past u64, so write it as text.
                let mut wide = binding(json!({"component":123_456_789,"class":power}));
                let text = String::from_utf8(wide[&home].clone()).unwrap().replace("123456789", "18446744073709551616");
                wide.insert(home.clone(), text.into_bytes());
                wide
            },
            binding(json!({"component":key})),
            binding(json!({"class":power})),
            binding(json!({"component":key,"class":null})),
            binding(json!({"component":key,"class":Uuid::nil()})),
            binding(json!({"component":key,"class":"Power"})),
            edited(&docs, &home, |value| {
                let list = if board { &mut value["component_class_bindings"] } else { &mut value["schematic_instances"]["component_class_bindings"] };
                *list = json!([{"component":key,"class":power},{"component":key,"class":power}]);
            }),
            edited(&docs, ENTRY_POINT, |project| {
                project.as_object_mut().unwrap().remove("component_classes");
            }),
            edited(&docs, ENTRY_POINT, |project| {
                project["component_classes"].as_object_mut().unwrap().remove("definitions");
            }),
            edited(&docs, ENTRY_POINT, |project| project["component_classes"]["extra"] = json!(1)),
            edited(&docs, ENTRY_POINT, |project| {
                let definitions = &mut project["component_classes"]["definitions"];
                definitions[1]["id"] = definitions[0]["id"].clone();
            }),
            edited(&docs, ENTRY_POINT, |project| project["component_classes"]["definitions"][0]["id"] = json!(Uuid::nil())),
        ];
        let mut reasons = vec![
            "invalid value: integer `-1`, expected u64",
            "invalid type: string",
            "invalid type: floating point",
            "missing field `class`",
            "missing field `component`",
            "invalid type: null, expected a formatted UUID",
            "must name existing components and classes",
            "UUID parsing failed",
            "sorted by component and class",
            "Component-class catalog requires project format 24",
            "missing field `definitions`",
            "unknown field `extra`",
            "Class IDs must be unique",
            "Class IDs must be unique",
        ];
        if let Some(path) = &path {
            cases.push(edited(&docs, path, |instances| {
                instances.as_object_mut().unwrap().remove("component_class_bindings");
            }));
            reasons.push("unsupported instances format");
        }
        assert_eq!(cases.len(), reasons.len());
        for (case, reason) in cases.iter().zip(reasons) {
            refused_on_read(case, reason);
        }
    }
}

/// Command parameters outside the published shapes are refused without changing the document.
#[test]
fn command_parameter_edges_are_refused() {
    let (mut s, components) = parts(1, false);
    let class = create(&mut s, "Power");
    let before = s.snapshot().unwrap();
    let set = "unique non-nil class UUIDs";
    for (params, reason) in [
        (json!({"component":components[0],"classes":[Uuid::nil()]}), set),
        (json!({"component":components[0]}), "needs `classes`"),
        (json!({"component":components[0],"classes":null}), set),
        (json!({"component":components[0],"classes":class}), set),
        (json!({"component":components[0],"classes":["Power"]}), set),
        (json!({"classes":[class]}), "needs `component`"),
        (json!({"component":null,"classes":[class]}), "`component` must be a string"),
        (json!({"component":1,"classes":[class]}), "`component` must be a string"),
    ] {
        refuse(&mut s, SET_CLASSES, params, reason);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let uuid = "must identify a class by non-nil UUID";
    for (params, reason) in
        [(json!({"class":Uuid::nil(),"name":"X"}), uuid), (json!({"name":"X"}), "needs `class`"), (json!({"class":class}), "needs `name`")]
    {
        refuse(&mut s, UPDATE, params, reason);
    }
    for (params, reason) in [(json!({"class":Uuid::nil()}), uuid), (json!({}), "needs `class`"), (json!({"class":"Power"}), uuid)] {
        refuse(&mut s, REMOVE, params, reason);
    }
    for (params, reason) in [
        (json!({"component":null}), "`component` must be a string"),
        (json!({"component":"R1"}), "Expected a component ID"),
        (json!({"component":"component-999"}), "Unknown component."),
    ] {
        let refusal = s.execute(INSPECT_CLASSES, &params).unwrap_err().to_string();
        assert!(refusal.contains(reason), "{params}: expected {reason:?}, got {refusal:?}");
    }
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Project/24 is a superset of project/23: component classes with members round-trip beside a
/// net class with an assignment, a project parameter, a no-connect marker and generated net
/// numbers. A board catalog without members over that registry keeps instances/4.
#[test]
fn component_classes_round_trip_with_every_project_23_record() {
    for board in [false, true] {
        let (mut s, components) = parts(3, board);
        let pins: Vec<_> = s.project().unwrap().symbols().iter().take(2).map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"})).collect();
        apply(&mut s, crate::CONNECT_PINS, json!({"from":pins[0],"to":pins[1]}));
        apply(&mut s, crate::parameters::SET, json!({"scope":"project","name":"Revision","value":"B"}));
        apply(&mut s, crate::no_connect::SET, json!({"target":{"part":components[2],"terminal":"2"},"marked":true}));
        apply(&mut s, crate::net_classes::CREATE, json!({"name":"Power nets","rules":{"clearance_nm":500_000}}));
        let net_class = s.project().unwrap().net_classes().definitions[0].id;
        let project = s.project().unwrap();
        let anchor = crate::nets::Anchor {
            kind: crate::nets::AnchorKind::Terminal,
            project: project.id,
            scope: project.net_scope(),
            occurrence: Uuid::nil(),
            object: component_key(&components[0]).unwrap(),
            terminal: "1".into(),
        };
        apply(&mut s, crate::nets::SET_CLASSES, json!({"target":crate::nets::ClassTarget::Derived{anchor},"classes":[net_class]}));
        let path = s.project().unwrap().instances_document_path();
        assert_eq!(path.is_some(), board);

        // A catalog without members leaves a board's registry-bearing instance header at /4.
        let empty = create(&mut s, "Unused");
        let docs = round_trip(&s);
        assert_eq!(document(&docs, ENTRY_POINT)["format"], "pcbcraft-project/24");
        if let Some(path) = &path {
            assert_eq!(document(&docs, path)["format"], "pcbcraft-instances/4");
            assert!(document(&docs, path).get("net_registry").is_some());
        }
        apply(&mut s, REMOVE, json!({"class":empty}));

        let power = create(&mut s, "Power");
        for component in &components {
            apply(&mut s, SET_CLASSES, json!({"component":component,"classes":[power]}));
        }
        let project = s.snapshot().unwrap();
        let docs = round_trip(&s);
        let entry = document(&docs, ENTRY_POINT);
        assert_eq!(entry["format"], "pcbcraft-project/24");
        let home = match &path {
            Some(path) => {
                let instances = document(&docs, path);
                assert_eq!(instances["format"], "pcbcraft-instances/5");
                instances
            }
            None => entry["schematic_instances"].clone(),
        };
        assert_eq!(entry["parameters"], json!({"Revision":"B"}));
        assert!(entry["net_classes"]["definitions"][0]["id"] == json!(net_class));
        assert!(String::from_utf8(docs[ENTRY_POINT].clone()).unwrap().contains("no_connects"));
        assert!(home.get("net_registry").is_some() && home.get("component_class_bindings").is_some());
        let reopened = decode_documents(&docs).unwrap();
        assert_eq!(reopened.parameters(), project.parameters());
        assert_eq!(reopened.net_classes(), project.net_classes());
        assert_eq!(reopened.net_registry, project.net_registry);
        assert!(reopened.net_registry.has_generated_numbers());
        assert_eq!(reopened.component_classes(), project.component_classes());
        assert_eq!(memberships(&reopened), memberships(&project));
        let marked = |project: &Project| project.components.values().filter(|part| !part.no_connects.is_empty()).count();
        assert_eq!((marked(&reopened), marked(&project)), (1, 1));
        assert_eq!(encode_documents(reopened).unwrap(), docs);
    }
}

/// Readers accept catalog definitions in any order and sort them by UUID, so a reversed file
/// decodes to the same classes and re-encodes to the original sorted bytes.
#[test]
fn reversed_catalog_definitions_decode_sorted() {
    let (mut s, _) = parts(0, false);
    create(&mut s, "First");
    create(&mut s, "Second");
    let docs = round_trip(&s);
    let reversed = edited(&docs, ENTRY_POINT, |project| project["component_classes"]["definitions"].as_array_mut().unwrap().reverse());
    let sorted = document(&docs, ENTRY_POINT)["component_classes"]["definitions"].clone();
    assert_ne!(document(&reversed, ENTRY_POINT)["component_classes"]["definitions"], sorted);
    let reopened = decode_documents(&reversed).unwrap();
    assert_eq!(reopened.component_classes(), s.project().unwrap().component_classes());
    assert_eq!(encode_documents(reopened).unwrap(), docs);
}

/// Edits that keep the component map, such as moving a symbol, skip the membership scan and
/// leave the stored memberships untouched; deleting a component still prunes them.
#[test]
fn geometry_edits_leave_memberships_untouched() {
    let (mut s, components) = parts(2, true);
    let class = create(&mut s, "Power");
    for component in &components {
        apply(&mut s, SET_CLASSES, json!({"component":component,"classes":[class]}));
    }
    let before = s.snapshot().unwrap();
    let symbol = before.symbols()[0].id();
    let revision = s.revision();
    apply(&mut s, crate::MOVE_SYMBOL, json!({"symbol":symbol,"x_nm":0,"y_nm":25_400_000}));
    assert!(s.revision() > revision);
    let after = s.snapshot().unwrap();
    assert!(Arc::ptr_eq(&before.components, &after.components));
    assert!(Arc::ptr_eq(&before.component_class_bindings, &after.component_class_bindings));
    assert_eq!(memberships(&after), memberships(&before));
    round_trip(&s);
}

/// Validation looks up each distinct class and component once, not once per binding: a full
/// catalog of 256 classes with every one of 16 components in every class (4,096 bindings)
/// costs 256 class lookups and 16 component lookups, and a dangling class is still refused
/// with the reason. The same memberships pass through the command path and a save and reopen.
#[test]
fn membership_validation_looks_up_each_class_once() {
    use std::cell::Cell;
    let (mut s, components) = parts(16, false);
    for index in 0..MAX_CLASSES {
        apply(&mut s, CREATE, json!({"name":format!("Class {index}")}));
    }
    let classes: Vec<Uuid> = s.project().unwrap().component_classes().definitions.iter().map(|class| class.id).collect();
    for component in &components {
        apply(&mut s, SET_CLASSES, json!({"component":component,"classes":classes}));
    }
    let project = s.project().unwrap();
    let bindings = project.component_class_bindings();
    assert_eq!(bindings.len(), components.len() * MAX_CLASSES);
    let (class_lookups, component_lookups) = (Cell::new(0), Cell::new(0));
    let known: BTreeSet<Uuid> = classes.iter().copied().collect();
    let counted = |dangling: &[Binding]| {
        class_lookups.set(0);
        component_lookups.set(0);
        check_bindings(
            dangling,
            |_| {
                component_lookups.set(component_lookups.get() + 1);
                true
            },
            |class| {
                class_lookups.set(class_lookups.get() + 1);
                known.contains(&class)
            },
        )
    };
    counted(bindings).unwrap();
    assert_eq!((class_lookups.get(), component_lookups.get()), (MAX_CLASSES, components.len()));
    let mut dangling = bindings.to_vec();
    dangling.last_mut().unwrap().class = Uuid::from_u128(u128::MAX);
    dangling.sort();
    let refusal = counted(&dangling).unwrap_err();
    assert!(refusal.contains("existing components and classes"), "{refusal}");
    assert!(class_lookups.get() <= MAX_CLASSES + 1);
    project.validate_component_class_bindings(bindings).unwrap();
    let docs = round_trip(&s);
    assert_eq!(decode_documents(&docs).unwrap().component_class_bindings().len(), bindings.len());
}
