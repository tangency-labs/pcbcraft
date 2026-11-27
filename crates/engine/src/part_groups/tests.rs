//! Part-group acceptance through the shared command path and the native codecs.
use super::*;
use crate::{
    CommandError,
    CommandError::{CapExceeded, InvalidParams, StaleRevision, Unavailable},
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

/// The same request as `apply` at the current revision, refused with the given error variant
/// and a message containing `reason`, so a different cause cannot satisfy the assertion.
#[track_caller]
fn refuse(session: &mut Session, command: &str, mut params: Value, kind: fn(String) -> CommandError, reason: &str) -> String {
    params["expected_revision"] = json!(session.revision());
    let error = session.apply(command, &params).unwrap_err();
    let refusal = error.to_string();
    assert_eq!(error, kind(refusal.clone()), "{command} {params}: unexpected refusal kind");
    assert!(refusal.contains(reason), "{command} {params}: expected {reason:?}, got {refusal:?}");
    refusal
}

/// Decoding must fail for `reason`; any other failure or a success fails the test.
#[track_caller]
fn refused_on_read(documents: &crate::persistence::Documents, reason: &str) {
    let refusal = decode_documents(documents).err().unwrap_or_else(|| panic!("expected {reason:?}, the documents decoded"));
    assert!(refusal.contains(reason), "expected {reason:?}, got {refusal:?}");
}

/// A project with `count` placed resistors, optionally with a board, and their part-instance IDs.
fn parts(count: usize, board: bool) -> (Session, Vec<String>) {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({"create_board":board}));
    for index in 0..count {
        apply(&mut session, crate::PLACE_RESISTOR, json!({"x_nm":index as i64*25_400_000,"y_nm":0}));
    }
    let part_instances = session.project().unwrap().part_instance_addresses().collect();
    (session, part_instances)
}

/// Create a group and return its UUID from inspection, never from its name alone.
fn create(session: &mut Session, name: &str) -> Uuid {
    apply(session, CREATE, json!({"name":name}));
    let report = session.execute(INSPECT, &json!({})).unwrap();
    let groups = report["part_groups"]["groups"].as_array().unwrap();
    serde_json::from_value(groups.iter().rev().find(|group| group["name"] == name).unwrap()["id"].clone()).unwrap()
}

/// Every membership as (part-instance address, group), the form the done test compares.
fn memberships(project: &Project) -> Vec<(String, Uuid)> {
    project.part_group_memberships().iter().map(|membership| (part_instance_address(membership.part_instance), membership.group)).collect()
}

/// Parse one saved JSON document.
fn document(docs: &crate::persistence::Documents, path: &str) -> Value {
    serde_json::from_slice(&docs[path]).unwrap()
}

/// Change a saved field without pretty-print growth crossing the native byte limits first.
fn edited(docs: &crate::persistence::Documents, path: &str, edit: impl FnOnce(&mut Value)) -> crate::persistence::Documents {
    let mut value = document(docs, path);
    edit(&mut value);
    let mut docs = docs.clone();
    docs.insert(path.into(), serde_json::to_vec(&value).unwrap());
    assert!(docs.values().all(|bytes| bytes.len() <= crate::persistence::MAX_DOCUMENT_BYTES), "mutation exceeds a document byte limit");
    assert!(docs.values().map(Vec::len).sum::<usize>() <= crate::persistence::MAX_DOCUMENTS_BYTES, "mutation exceeds the total byte limit");
    docs
}

/// Save and reopen through folders and the packed archive, returning the folder documents.
fn round_trip(session: &Session) -> crate::persistence::Documents {
    let project = session.snapshot().unwrap();
    let docs = encode_documents(project.clone()).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert_eq!(reopened.part_groups(), project.part_groups());
    assert_eq!(memberships(&reopened), memberships(&project));
    assert_eq!(encode_documents(reopened).unwrap(), docs);
    assert_eq!(decode_packed(&encode_packed(project).unwrap()).unwrap().documents, docs);
    docs
}

/// Create a group, assign three part instances, save and reload: membership by part-instance ID
/// is unchanged, with the schematic-only and the board-owned instance home.
#[test]
fn three_memberships_survive_save_and_reload_by_part_instance_id() {
    for board in [false, true] {
        let (mut s, part_instances) = parts(3, board);
        // A numbered net puts the registry ahead of the memberships in the instance home.
        let pins: Vec<_> = s.project().unwrap().symbols().iter().take(2).map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"})).collect();
        apply(&mut s, crate::CONNECT_PINS, json!({"from":pins[0],"to":pins[1]}));
        assert_eq!(document(&encode_documents(s.snapshot().unwrap()).unwrap(), ENTRY_POINT)["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
        let power = create(&mut s, "Power");
        for part_instance in &part_instances {
            apply(&mut s, SET_GROUPS, json!({"part_instance":part_instance,"groups":[power]}));
        }
        let before = memberships(s.project().unwrap());
        assert_eq!(before, part_instances.iter().map(|address| (address.clone(), power)).collect::<Vec<_>>());
        let docs = round_trip(&s);
        let reopened = decode_documents(&docs).unwrap();
        assert_eq!(memberships(&reopened), before);
        let project = document(&docs, ENTRY_POINT);
        assert_eq!(project["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
        assert_eq!(project["part_groups"]["format"], FORMAT);
        assert_eq!(reopened.net_registry, s.project().unwrap().net_registry);
        assert!(reopened.net_registry.has_generated_numbers());
        let home = match s.project().unwrap().instances_document_path() {
            Some(path) => {
                assert!(board);
                let instances = document(&docs, &path);
                assert_eq!(instances["format"], crate::persistence::CURRENT_INSTANCES_FORMAT);
                assert!(project.get("schematic_instances").is_none());
                instances
            }
            None => {
                assert!(!board);
                project["schematic_instances"].clone()
            }
        };
        let keys: Vec<_> = part_instances.iter().map(|address| part_instance_key(address).unwrap()).collect();
        assert_eq!(home["part_group_memberships"], json!(keys.iter().map(|key| json!({"part_instance":key,"group":power})).collect::<Vec<_>>()));
        let report = s.execute(INSPECT_GROUPS, &json!({"part_instance":part_instances[1]})).unwrap();
        assert_eq!(report["part_instance_memberships"], json!([{"part_instance":part_instances[1],"groups":[power]}]));
    }
}

/// Both sides of the old scale-test count remain in the legacy envelope while their bytes fit.
/// Reopening a valid 4097-membership file must not silently choose a different encoding.
#[test]
fn legacy_memberships_keep_their_bytes_above_four_thousand_records() {
    for board in [false, true] {
        let (mut s, ids) = parts(17, board);
        let groups: Vec<_> = (0..MAX_GROUPS).map(|index| create(&mut s, &format!("Group {index}"))).collect();
        for id in ids.iter().take(16) {
            apply(&mut s, SET_GROUPS, json!({"part_instance":id,"groups":groups}));
        }
        for count in [4096, 4097] {
            if count == 4097 {
                apply(&mut s, SET_GROUPS, json!({"part_instance":ids[16],"groups":[groups[0]]}));
            }
            assert_eq!(s.project().unwrap().part_group_memberships().len(), count);
            let docs = round_trip(&s);
            assert_eq!(document(&docs, ENTRY_POINT)["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
            if let Some(path) = s.project().unwrap().instances_document_path() {
                assert_eq!(document(&docs, &path)["format"], crate::persistence::CURRENT_INSTANCES_FORMAT);
            }
            let mut writer = crate::persistence::DocumentsEncoder::new(s.snapshot().unwrap());
            assert!((0..256).any(|_| {
                assert!(writer.step(1).unwrap().is_none());
                writer.grouped_rows_checked() == Some(0)
            }));
            for row in 1..=count.div_ceil(MAX_GROUPS) {
                assert!(writer.step(1).unwrap().is_none());
                assert_eq!(writer.grouped_rows_checked(), Some(row));
            }
            loop {
                if let Some(incremental) = writer.step(128).unwrap() {
                    assert_eq!(incremental, docs);
                    break;
                }
            }
        }
    }
}

/// Dense accepted membership, including the full 4096-by-256 space, saves through both native
/// homes under the existing byte limits. Canonical compact output is byte-identical after reopen.
#[test]
fn maximum_memberships_save_and_reload_in_both_storage_homes() {
    for board in [false, true] {
        let (mut s, _) = parts(0, board);
        for index in 0..crate::schematic::MAX_SYMBOLS {
            let spacing = 12 * crate::schematic::GRID_NM;
            apply(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":(index % 64) as i64*spacing,"y_nm":(index / 64) as i64*spacing}));
            if index % 64 == 63 {
                s.open_snapshot(s.snapshot().unwrap());
            }
        }
        let ids: Vec<_> = s.project().unwrap().part_instance_addresses().collect();
        let groups: Vec<_> = (0..MAX_GROUPS).map(|index| create(&mut s, &format!("Group {index}"))).collect();
        s.open_snapshot(s.snapshot().unwrap());
        for (index, id) in ids.iter().enumerate() {
            apply(&mut s, SET_GROUPS, json!({"part_instance":id,"groups":groups}));
            if index % 64 == 63 {
                s.open_snapshot(s.snapshot().unwrap());
            }
            if index == 1023 || index + 1 == ids.len() {
                let expected = (index + 1) * MAX_GROUPS;
                assert_eq!(s.project().unwrap().part_group_memberships().len(), expected);
                let docs = round_trip(&s);
                assert_eq!(document(&docs, ENTRY_POINT)["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
                assert!(docs.values().all(|bytes| bytes.len() <= crate::persistence::MAX_DOCUMENT_BYTES));
                assert!(docs.values().map(Vec::len).sum::<usize>() <= crate::persistence::MAX_DOCUMENTS_BYTES);
                let home_path = s.project().unwrap().instances_document_path();
                let home = home_path.as_deref().unwrap_or(ENTRY_POINT);
                let value = document(&docs, home);
                let scope = if board { &value } else { &value["schematic_instances"] };
                assert!(scope.get("part_group_memberships").is_none());
                assert_eq!(scope["compact_part_group_memberships"]["part_instances"].as_array().unwrap().len(), index + 1);
                assert!(serde_json::to_vec(&scope["compact_part_group_memberships"]).unwrap().len() < 4 * 1024 * 1024);
                if board {
                    assert_eq!(value["format"], crate::persistence::CURRENT_INSTANCES_FORMAT);
                }
                let downgraded = edited(&docs, ENTRY_POINT, |project| project["format"] = json!("pcbcraft-project/24"));
                if board {
                    refused_on_read(&downgraded, "unsupported instances format");
                } else {
                    assert_eq!(
                        decode_documents(&downgraded).unwrap_err(),
                        "compact_part_group_memberships is not allowed by this instance encoding."
                    );
                }
                for remove in [true, false] {
                    let malformed = edited(&docs, home, |value| {
                        let scope = if board { value } else { &mut value["schematic_instances"] };
                        if remove {
                            scope.as_object_mut().unwrap().remove("compact_part_group_memberships");
                        } else {
                            scope["part_group_memberships"] = json!([{"part_instance":part_instance_key(&ids[0]).unwrap(),"group":groups[0]}]);
                        }
                    });
                    if remove {
                        assert!(decode_documents(&malformed).unwrap().part_group_memberships().is_empty());
                        if board {
                            let legacy = edited(&malformed, home, |value| value["format"] = json!("pcbcraft-instances/7"));
                            assert_eq!(decode_documents(&legacy).unwrap_err(), "Missing required compact_part_group_memberships.");
                        }
                    } else {
                        assert_eq!(
                            decode_documents(&malformed).unwrap_err(),
                            "part_group_memberships and compact_part_group_memberships cannot both be present."
                        );
                    }
                }
                if board {
                    let downgraded = edited(&docs, home, |value| value["format"] = json!("pcbcraft-instances/5"));
                    refused_on_read(&downgraded, "unsupported instances format");
                }
                if index + 1 == ids.len() {
                    for foreign_group in [false, true] {
                        let foreign = edited(&docs, home, |value| {
                            let scope = if board { value } else { &mut value["schematic_instances"] };
                            let compact = &mut scope["compact_part_group_memberships"];
                            if foreign_group {
                                compact["groups"][0] = json!(Uuid::from_u128(1));
                            } else {
                                compact["part_instances"][0]["part_instance"] = json!(0);
                            }
                        });
                        refused_on_read(&foreign, "must name existing part instances and groups");
                    }
                }
            }
        }
        assert_eq!(s.project().unwrap().part_group_memberships().len(), MAX_MEMBERSHIPS);
        let snapshot = s.snapshot().unwrap();
        refuse(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}), CapExceeded, "already has 4096 symbols");
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    }
}

/// Adding or removing an empty catalog keeps the project header and board instance bytes.
/// A project with no catalog omits the optional field.
#[test]
fn untouched_projects_keep_their_bytes_and_headers() {
    for board in [false, true] {
        let (mut s, _) = parts(2, board);
        let untouched = encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(document(&untouched, ENTRY_POINT)["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
        assert!(document(&untouched, ENTRY_POINT).get("part_groups").is_none());
        let group = create(&mut s, "Connectors");
        let docs = round_trip(&s);
        assert_eq!(document(&docs, ENTRY_POINT)["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
        if let Some(path) = s.project().unwrap().instances_document_path() {
            assert_eq!(document(&docs, &path)["format"], document(&untouched, &path)["format"]);
            assert!(document(&docs, &path).get("part_group_memberships").is_none());
        } else {
            assert!(document(&docs, ENTRY_POINT)["schematic_instances"].get("part_group_memberships").is_none());
        }
        apply(&mut s, REMOVE, json!({"group":group}));
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), untouched);
        apply(&mut s, "edit.undo", json!({}));
        apply(&mut s, "edit.undo", json!({}));
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), untouched);
    }
}

/// Older headers carrying part-group records, explicit empties, foreign memberships and unknown
/// fields are refused rather than silently dropped.
#[test]
fn malformed_and_downgraded_documents_are_refused() {
    for board in [false, true] {
        let (mut s, part_instances) = parts(2, board);
        let group = create(&mut s, "Power");
        apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[0],"groups":[group]}));
        let docs = round_trip(&s);
        let path = s.project().unwrap().instances_document_path();
        let home = path.clone().unwrap_or_else(|| ENTRY_POINT.into());
        /// The membership list in this project's instance home.
        fn field(value: &mut Value, board: bool) -> &mut Value {
            if board { &mut value["part_group_memberships"] } else { &mut value["schematic_instances"]["part_group_memberships"] }
        }
        let set_list = |value: &mut Value, list: Value| *field(value, board) = list;
        let mut cases = vec![
            edited(&docs, ENTRY_POINT, |project| project["format"] = json!("pcbcraft-project/23")),
            edited(&docs, ENTRY_POINT, |project| project["part_groups"]["groups"] = json!([])),
            edited(&docs, ENTRY_POINT, |project| project["part_groups"] = Value::Null),
            edited(&docs, ENTRY_POINT, |project| project["part_groups"]["format"] = json!("pcbcraft-part-groups/2")),
            edited(&docs, ENTRY_POINT, |project| project["part_groups"]["groups"][0]["rules"] = json!({})),
            edited(&docs, &home, |value| set_list(value, json!([]))),
            edited(&docs, &home, |value| set_list(value, Value::Null)),
            edited(&docs, &home, |value| set_list(value, json!([{"part_instance":999,"group":group}]))),
            edited(&docs, &home, |value| {
                set_list(value, json!([{"part_instance":part_instance_key(&part_instances[0]),"group":Uuid::from_u128(7)}]))
            }),
            edited(&docs, &home, |value| set_list(value, json!([{"part_instance":part_instance_key(&part_instances[0]),"group":group,"origin":1}]))),
            edited(&docs, &home, |value| {
                set_list(
                    value,
                    json!([{"part_instance":part_instance_key(&part_instances[1]),"group":group},{"part_instance":part_instance_key(&part_instances[0]),"group":group}]),
                )
            }),
        ];
        let mut reasons = vec![
            "part_groups not allowed in pcbcraft-project/23.",
            "Empty part-group catalogs must be omitted",
            "invalid type: null, expected struct Catalog",
            "Unsupported part-group format",
            "unknown field `rules`",
            "must be omitted or nonempty",
            "invalid type: null, expected a sequence",
            "must name existing part instances and groups",
            "must name existing part instances and groups",
            "unknown field `origin`",
            "sorted by part instance and group",
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

/// Catalog edits are revisioned and atomic; removing a group with members is refused and
/// names them; no-op requests leave history and revision untouched; undo restores both.
#[test]
fn catalog_and_membership_edits_are_atomic_and_undoable() {
    let (mut s, part_instances) = parts(3, false);
    let power = create(&mut s, "Power");
    let logic = create(&mut s, "Logic");
    apply(&mut s, UPDATE, json!({"group":power,"name":"Supply"}));
    let supply = s.project().unwrap().part_groups().get(power).unwrap().clone();
    assert_eq!((supply.name.as_str(), supply.revision), ("Supply", 2));
    let revision = s.revision();
    apply(&mut s, UPDATE, json!({"group":power,"name":"Supply"}));
    apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[0],"groups":[]}));
    assert_eq!(s.revision(), revision);

    refuse(&mut s, SET_GROUPS, json!({"part_instance":part_instances[0],"groups":[logic,power,power]}), InvalidParams, "unique non-nil class UUIDs");
    assert_eq!(s.revision(), revision);
    apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[0],"groups":[logic,power]}));
    apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[2],"groups":[power]}));
    let assigned = s.snapshot().unwrap();
    let revision = s.revision();
    apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[0],"groups":[power,logic]}));
    assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
    let mut expected = vec![power, logic];
    expected.sort();
    assert_eq!(s.project().unwrap().groups_of_part_instance(part_instance_key(&part_instances[0]).unwrap()).collect::<Vec<_>>(), expected);

    let refusal = refuse(&mut s, REMOVE, json!({"group":power}), InvalidParams, "still has members");
    assert!(refusal.contains(&part_instances[0]) && refusal.contains(&part_instances[2]) && !refusal.contains(&part_instances[1]), "{refusal}");
    for (command, params, reason) in [
        (SET_GROUPS, json!({"part_instance":part_instance_address(999),"groups":[power]}), "Unknown part instance."),
        (SET_GROUPS, json!({"part_instance":part_instances[1],"groups":[Uuid::from_u128(5)]}), "Unknown part group."),
        (SET_GROUPS, json!({"part_instance":"R1","groups":[power]}), "Expected a part-instance ID"),
        (UPDATE, json!({"group":power,"name":" padded"}), "Group name must contain"),
        (UPDATE, json!({"group":Uuid::from_u128(5),"name":"Other"}), "Unknown part group."),
        (REMOVE, json!({"group":Uuid::from_u128(5)}), "Unknown part group."),
    ] {
        refuse(&mut s, command, params, InvalidParams, reason);
        assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
    }
    let stale = json!({"part_instance":part_instances[1],"groups":[power],"expected_revision":s.revision() - 1});
    let error = s.apply(SET_GROUPS, &stale).unwrap_err();
    assert!(matches!(error, StaleRevision { current } if current == revision), "{error:?}");
    assert_eq!(error.details(), json!({"current_revision":revision}));
    let refusal = error.to_string();
    assert!(refusal.contains("project changed"), "{refusal}");
    assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));

    apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[2],"groups":[]}));
    apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[0],"groups":[logic]}));
    apply(&mut s, REMOVE, json!({"group":power}));
    assert!(s.project().unwrap().part_groups().get(power).is_none());
    apply(&mut s, "edit.undo", json!({}));
    apply(&mut s, "edit.undo", json!({}));
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(memberships(s.project().unwrap()), memberships(&assigned));
    apply(&mut s, "edit.redo", json!({}));
    assert_eq!(s.project().unwrap().groups_of_part_instance(part_instance_key(&part_instances[2]).unwrap()).count(), 0);
    let report = s.execute(INSPECT_GROUPS, &json!({})).unwrap();
    assert_eq!(report["part_instance_memberships"].as_array().unwrap().len(), 1);
}

/// Deleting a part instance removes its memberships in the same history step; undo restores them.
#[test]
fn deleting_a_part_instance_takes_its_memberships_with_it() {
    let (mut s, part_instances) = parts(2, false);
    let group = create(&mut s, "Power");
    for part_instance in &part_instances {
        apply(&mut s, SET_GROUPS, json!({"part_instance":part_instance,"groups":[group]}));
    }
    let assigned = memberships(s.project().unwrap());
    let symbol = s.project().unwrap().symbols()[0].id();
    apply(&mut s, crate::DELETE_SYMBOL, json!({"symbol":symbol}));
    let remaining: Vec<_> = s.project().unwrap().part_instance_addresses().collect();
    assert_eq!(remaining.len(), 1);
    assert!(part_instances.contains(&remaining[0]));
    assert_eq!(memberships(s.project().unwrap()), vec![(remaining[0].clone(), group)]);
    round_trip(&s);
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(memberships(s.project().unwrap()), assigned);
}

/// The single accessor pair is the only conversion between published addresses and stored keys.
#[test]
fn part_instance_keys_round_trip_through_addresses() {
    // The published address form is pinned literally: saved projects and reports use it.
    assert_eq!(part_instance_key("component-42"), Some(42));
    assert_eq!(part_instance_address(42), "component-42");
    let non_numeric = format!("{PART_INSTANCE_ADDRESS_PREFIX}x");
    for invalid in ["R1", "component-", non_numeric.as_str(), "42"] {
        assert_eq!(part_instance_key(invalid), None);
    }
}

/// Names are accepted up to 200 trimmed characters; the first character past the limit, blank,
/// padded and control-containing names are refused on create, update and read.
#[test]
fn group_names_are_bounded_on_each_side_of_the_limit() {
    let (mut s, _) = parts(0, false);
    let longest = "A".repeat(200);
    let group = create(&mut s, &longest);
    assert_eq!(s.project().unwrap().part_groups().get(group).unwrap().name, longest);
    let docs = round_trip(&s);
    let assigned = s.snapshot().unwrap();
    for name in ["A".repeat(201), String::new(), " A".into(), "A ".into(), "A\nB".into()] {
        refuse(&mut s, CREATE, json!({"name":name}), InvalidParams, "Group name must contain 1–200");
        refuse(&mut s, UPDATE, json!({"group":group,"name":name}), InvalidParams, "Group name must contain 1–200");
        assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
        let saved = edited(&docs, ENTRY_POINT, |project| project["part_groups"]["groups"][0]["name"] = json!(name));
        refused_on_read(&saved, "Group name must contain 1–200");
    }
    for (params, reason) in
        [(json!({}), "needs `name`"), (json!({"name":null}), "`name` must be a string"), (json!({"name":7}), "`name` must be a string")]
    {
        refuse(&mut s, CREATE, params, InvalidParams, reason);
    }
}

/// A catalog holds at most 256 groups: the 256th is created, saved and reopened; the 257th is
/// refused by the command and by the reader.
#[test]
fn catalog_size_is_bounded_on_each_side_of_the_limit() {
    let (mut s, _) = parts(0, false);
    for index in 0..MAX_GROUPS {
        apply(&mut s, CREATE, json!({"name":format!("Group {index}")}));
    }
    assert_eq!(s.project().unwrap().part_groups().groups.len(), MAX_GROUPS);
    let docs = round_trip(&s);
    let full = s.snapshot().unwrap();
    refuse(&mut s, CREATE, json!({"name":"One more"}), Unavailable, "catalog is full");
    assert!(Arc::ptr_eq(&full, &s.snapshot().unwrap()));
    let overfull = edited(&docs, ENTRY_POINT, |project| {
        project["part_groups"]["groups"].as_array_mut().unwrap().push(json!({"id":Uuid::from_u128(u128::MAX),"revision":1,"name":"Extra"}));
    });
    refused_on_read(&overfull, "group limit exceeded");
}

/// Saved revisions run from 1 to `u32::MAX`. The largest is read, and a later rename is then
/// refused as exhausted rather than wrapping; zero, negative and wider values are refused.
#[test]
fn group_revisions_are_bounded_on_each_side_of_the_limit() {
    let (mut s, _) = parts(0, false);
    let group = create(&mut s, "Power");
    let docs = round_trip(&s);
    let revision = |value: Value| edited(&docs, ENTRY_POINT, |project| project["part_groups"]["groups"][0]["revision"] = value);
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
    assert_eq!(last.part_groups().get(group).unwrap().revision, u32::MAX);
    s.open_snapshot(last);
    refuse(&mut s, UPDATE, json!({"group":group,"name":"Supply"}), Unavailable, "revision exhausted");
    assert_eq!(s.project().unwrap().part_groups().get(group).unwrap().name, "Power");
}

/// Saved records with missing, null, wrong-typed or out-of-range fields, missing required
/// records, duplicate identities and unknown fields are refused by the reader.
#[test]
fn saved_record_edges_are_refused() {
    for board in [false, true] {
        let (mut s, part_instances) = parts(2, board);
        let power = create(&mut s, "Power");
        create(&mut s, "Logic");
        apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[0],"groups":[power]}));
        let docs = round_trip(&s);
        let path = s.project().unwrap().instances_document_path();
        let home = path.clone().unwrap_or_else(|| ENTRY_POINT.into());
        let key = part_instance_key(&part_instances[0]).unwrap();
        let membership = |record: Value| {
            edited(&docs, &home, |value| {
                let list = if board { &mut value["part_group_memberships"] } else { &mut value["schematic_instances"]["part_group_memberships"] };
                *list = json!([record]);
            })
        };
        let mut cases = vec![
            membership(json!({"part_instance":-1,"group":power})),
            membership(json!({"part_instance":part_instances[0],"group":power})),
            {
                // serde_json values cannot hold the first integer past u64, so write it as text.
                let mut wide = membership(json!({"part_instance":123_456_789,"group":power}));
                let text = String::from_utf8(wide[&home].clone()).unwrap().replace("123456789", "18446744073709551616");
                wide.insert(home.clone(), text.into_bytes());
                wide
            },
            membership(json!({"part_instance":key})),
            membership(json!({"group":power})),
            membership(json!({"part_instance":key,"group":null})),
            membership(json!({"part_instance":key,"group":Uuid::nil()})),
            membership(json!({"part_instance":key,"group":"Power"})),
            edited(&docs, &home, |value| {
                let list = if board { &mut value["part_group_memberships"] } else { &mut value["schematic_instances"]["part_group_memberships"] };
                *list = json!([{"part_instance":key,"group":power},{"part_instance":key,"group":power}]);
            }),
            edited(&docs, ENTRY_POINT, |project| {
                project.as_object_mut().unwrap().remove("part_groups");
            }),
            edited(&docs, ENTRY_POINT, |project| {
                project["part_groups"].as_object_mut().unwrap().remove("groups");
            }),
            edited(&docs, ENTRY_POINT, |project| project["part_groups"]["extra"] = json!(1)),
            edited(&docs, ENTRY_POINT, |project| {
                let groups = &mut project["part_groups"]["groups"];
                groups[1]["id"] = groups[0]["id"].clone();
            }),
            edited(&docs, ENTRY_POINT, |project| project["part_groups"]["groups"][0]["id"] = json!(Uuid::nil())),
        ];
        let mut reasons = vec![
            "invalid value: integer `-1`, expected u64",
            "invalid type: string",
            "invalid type: floating point",
            "missing field `group`",
            "missing field `part_instance`",
            "invalid type: null, expected a formatted UUID",
            "must name existing part instances and groups",
            "UUID parsing failed",
            "sorted by part instance and group",
            "must name existing part instances and groups",
            "missing field `groups`",
            "unknown field `extra`",
            "Group IDs must be unique",
            "Group IDs must be unique",
        ];
        if let Some(path) = &path {
            cases.push(edited(&docs, path, |instances| {
                instances["format"] = json!("pcbcraft-instances/5");
                instances.as_object_mut().unwrap().remove("part_group_memberships");
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
    let (mut s, part_instances) = parts(1, false);
    let group = create(&mut s, "Power");
    let before = s.snapshot().unwrap();
    let set = "unique non-nil class UUIDs";
    for (params, reason) in [
        (json!({"part_instance":part_instances[0],"groups":[Uuid::nil()]}), set),
        (json!({"part_instance":part_instances[0]}), "needs `groups`"),
        (json!({"part_instance":part_instances[0],"groups":null}), set),
        (json!({"part_instance":part_instances[0],"groups":group}), set),
        (json!({"part_instance":part_instances[0],"groups":["Power"]}), set),
        (json!({"groups":[group]}), "needs `part_instance`"),
        (json!({"part_instance":null,"groups":[group]}), "`part_instance` must be a string"),
        (json!({"part_instance":1,"groups":[group]}), "`part_instance` must be a string"),
    ] {
        refuse(&mut s, SET_GROUPS, params, InvalidParams, reason);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let uuid = "must identify a class by non-nil UUID";
    for (params, reason) in
        [(json!({"group":Uuid::nil(),"name":"X"}), uuid), (json!({"name":"X"}), "needs `group`"), (json!({"group":group}), "needs `name`")]
    {
        refuse(&mut s, UPDATE, params, InvalidParams, reason);
    }
    for (params, reason) in [(json!({"group":Uuid::nil()}), uuid), (json!({}), "needs `group`"), (json!({"group":"Power"}), uuid)] {
        refuse(&mut s, REMOVE, params, InvalidParams, reason);
    }
    for (params, reason) in [
        (json!({"part_instance":null}), "`part_instance` must be a string"),
        (json!({"part_instance":"R1"}), "Expected a part-instance ID"),
        (json!({"part_instance":part_instance_address(999)}), "Unknown part instance."),
    ] {
        let error = s.execute(INSPECT_GROUPS, &params).unwrap_err();
        assert!(matches!(error, InvalidParams(_)), "{error:?}");
        let refusal = error.to_string();
        assert!(refusal.contains(reason), "{params}: expected {reason:?}, got {refusal:?}");
    }
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Expanded and compact memberships preserve a net class with an assignment, a project
/// parameter, a no-connect marker and generated net numbers in both storage homes.
/// A board catalog without members keeps the same instance format.
#[test]
fn part_groups_round_trip_with_every_project_23_record() {
    for board in [false, true] {
        let (mut s, part_instances) = parts(3, board);
        let pins: Vec<_> = s.project().unwrap().symbols().iter().take(2).map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"})).collect();
        apply(&mut s, crate::CONNECT_PINS, json!({"from":pins[0],"to":pins[1]}));
        apply(&mut s, crate::parameters::SET, json!({"scope":"project","name":"Revision","value":"B"}));
        apply(&mut s, crate::no_connect::SET, json!({"target":{"part":part_instances[2],"terminal":"2"},"marked":true}));
        apply(&mut s, crate::net_classes::CREATE, json!({"name":"Power nets","rules":{"clearance_nm":500_000}}));
        let net_class = s.project().unwrap().net_classes().definitions[0].id;
        let project = s.project().unwrap();
        let anchor = crate::nets::Anchor {
            kind: crate::nets::AnchorKind::Terminal,
            project: project.id,
            scope: project.net_scope(),
            occurrence: Uuid::nil(),
            object: part_instance_key(&part_instances[0]).unwrap(),
            terminal: "1".into(),
        };
        apply(&mut s, crate::nets::SET_CLASSES, json!({"target":crate::nets::ClassTarget::Derived{anchor},"classes":[net_class]}));
        let path = s.project().unwrap().instances_document_path();
        assert_eq!(path.is_some(), board);

        // A catalog without members leaves the registry and its format in place.
        let empty = create(&mut s, "Unused");
        let docs = round_trip(&s);
        assert_eq!(document(&docs, ENTRY_POINT)["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
        if let Some(path) = &path {
            assert_eq!(document(&docs, path)["format"], crate::persistence::CURRENT_INSTANCES_FORMAT);
            assert!(document(&docs, path).get("net_registry").is_some());
        }
        apply(&mut s, REMOVE, json!({"group":empty}));

        let power = create(&mut s, "Power");
        for part_instance in &part_instances {
            apply(&mut s, SET_GROUPS, json!({"part_instance":part_instance,"groups":[power]}));
        }
        for compact in [false, true] {
            if compact {
                let mut project = (*s.snapshot().unwrap()).clone();
                project.prefer_compact_part_groups = true;
                s.open_snapshot(Arc::new(project));
            }
            let project = s.snapshot().unwrap();
            let docs = round_trip(&s);
            let entry = document(&docs, ENTRY_POINT);
            assert_eq!(entry["format"], crate::persistence::CURRENT_PROJECT_FORMAT);
            let home = match &path {
                Some(path) => {
                    let instances = document(&docs, path);
                    assert_eq!(instances["format"], crate::persistence::CURRENT_INSTANCES_FORMAT);
                    instances
                }
                None => entry["schematic_instances"].clone(),
            };
            assert_eq!(entry["parameters"], json!({"Revision":"B"}));
            assert!(entry["net_classes"]["definitions"][0]["id"] == json!(net_class));
            assert!(String::from_utf8(docs[ENTRY_POINT].clone()).unwrap().contains("no_connects"));
            assert!(home.get("net_registry").is_some());
            assert!(home.get(if compact { "compact_part_group_memberships" } else { "part_group_memberships" }).is_some());
            let reopened = decode_documents(&docs).unwrap();
            assert_eq!(reopened.parameters(), project.parameters());
            assert_eq!(reopened.net_classes(), project.net_classes());
            assert_eq!(reopened.net_registry, project.net_registry);
            assert!(reopened.net_registry.has_generated_numbers());
            assert_eq!(reopened.part_groups(), project.part_groups());
            assert_eq!(memberships(&reopened), memberships(&project));
            let marked = |project: &Project| project.part_instance_map().values().filter(|part| !part.no_connects.is_empty()).count();
            assert_eq!((marked(&reopened), marked(&project)), (1, 1));
            assert_eq!(encode_documents(reopened.clone()).unwrap(), docs);
            if let Some(path) = &path {
                let mut cleared = Session::default();
                cleared.open_snapshot(reopened);
                for part_instance in &part_instances {
                    apply(&mut cleared, SET_GROUPS, json!({"part_instance":part_instance,"groups":[]}));
                }
                let cleared_docs = round_trip(&cleared);
                let records = document(&cleared_docs, path);
                assert_eq!(records["format"], crate::persistence::CURRENT_INSTANCES_FORMAT);
                assert!(records.get("part_group_memberships").is_none());
                assert!(records.get("compact_part_group_memberships").is_none());
                assert_eq!(records["net_registry"], home["net_registry"]);
            }
        }
    }
}

/// Readers accept catalog entries in any order and sort them by UUID, so a reversed file
/// decodes to the same groups and re-encodes to the original sorted bytes.
#[test]
fn reversed_catalog_entries_decode_sorted() {
    let (mut s, _) = parts(0, false);
    create(&mut s, "First");
    create(&mut s, "Second");
    let docs = round_trip(&s);
    let reversed = edited(&docs, ENTRY_POINT, |project| project["part_groups"]["groups"].as_array_mut().unwrap().reverse());
    let sorted = document(&docs, ENTRY_POINT)["part_groups"]["groups"].clone();
    assert_ne!(document(&reversed, ENTRY_POINT)["part_groups"]["groups"], sorted);
    let reopened = decode_documents(&reversed).unwrap();
    assert_eq!(reopened.part_groups(), s.project().unwrap().part_groups());
    assert_eq!(encode_documents(reopened).unwrap(), docs);
}

/// Edits that keep the part-instance map, such as moving a symbol, skip the membership scan and
/// leave the stored memberships untouched; deleting a part instance still prunes them.
#[test]
fn geometry_edits_leave_memberships_untouched() {
    let (mut s, part_instances) = parts(2, true);
    let group = create(&mut s, "Power");
    for part_instance in &part_instances {
        apply(&mut s, SET_GROUPS, json!({"part_instance":part_instance,"groups":[group]}));
    }
    let before = s.snapshot().unwrap();
    let symbol = before.symbols()[0].id();
    let revision = s.revision();
    apply(&mut s, crate::MOVE_SYMBOL, json!({"symbol":symbol,"x_nm":0,"y_nm":25_400_000}));
    assert!(s.revision() > revision);
    let after = s.snapshot().unwrap();
    assert!(before.shares_part_instances(&after));
    assert!(Arc::ptr_eq(&before.part_group_memberships, &after.part_group_memberships));
    assert_eq!(memberships(&after), memberships(&before));
    round_trip(&s);
}

/// Validation looks up each distinct group and part instance once, not once per membership: a
/// full catalog of 256 groups with every one of 16 part instances in every group (4,096
/// memberships) costs 256 group lookups and 16 part-instance lookups, and a dangling group is refused
/// with the reason. The same memberships pass through the command path and a save and reopen.
#[test]
fn membership_validation_looks_up_each_group_once() {
    use std::cell::Cell;
    let (mut s, part_instances) = parts(16, false);
    for index in 0..MAX_GROUPS {
        apply(&mut s, CREATE, json!({"name":format!("Group {index}")}));
    }
    let groups: Vec<Uuid> = s.project().unwrap().part_groups().groups.iter().map(|group| group.id).collect();
    for part_instance in &part_instances {
        apply(&mut s, SET_GROUPS, json!({"part_instance":part_instance,"groups":groups}));
    }
    let project = s.project().unwrap();
    let stored = project.part_group_memberships();
    assert_eq!(stored.len(), part_instances.len() * MAX_GROUPS);
    let (group_lookups, part_instance_lookups) = (Cell::new(0), Cell::new(0));
    let known: BTreeSet<Uuid> = groups.iter().copied().collect();
    let counted = |records: &[Membership]| {
        group_lookups.set(0);
        part_instance_lookups.set(0);
        check_memberships(
            records.iter().copied(),
            |_| {
                part_instance_lookups.set(part_instance_lookups.get() + 1);
                true
            },
            |group| {
                group_lookups.set(group_lookups.get() + 1);
                known.contains(&group)
            },
        )
    };
    counted(&stored.to_records()).unwrap();
    assert_eq!((group_lookups.get(), part_instance_lookups.get()), (MAX_GROUPS, part_instances.len()));
    let mut dangling = stored.to_records();
    dangling.last_mut().unwrap().group = Uuid::from_u128(u128::MAX);
    dangling.sort();
    let refusal = counted(&dangling).unwrap_err();
    assert!(refusal.contains("existing part instances and groups"), "{refusal}");
    assert!(group_lookups.get() <= MAX_GROUPS + 1);
    project.validate_part_group_memberships(stored.iter()).unwrap();
    let docs = round_trip(&s);
    assert_eq!(decode_documents(&docs).unwrap().part_group_memberships().len(), stored.len());
}

/// Setting one part instance's groups builds that instance's set and shares every other set with
/// the previous snapshot, which Undo retains, so the whole history holds one set per edit; undo
/// returns the earlier shared sets. Only set allocations are counted: each edit also copies the
/// index of grouped part instances (one key and one pointer each), which this test does not measure.
#[test]
fn membership_edits_share_untouched_sets_with_history() {
    let (mut s, part_instances) = parts(3, false);
    let (power, signal) = (create(&mut s, "Power"), create(&mut s, "Signal"));
    for part_instance in &part_instances {
        apply(&mut s, SET_GROUPS, json!({"part_instance":part_instance,"groups":[power, signal]}));
    }
    let keys: Vec<_> = part_instances.iter().map(|address| part_instance_key(address).unwrap()).collect();
    let before = s.snapshot().unwrap();
    apply(&mut s, SET_GROUPS, json!({"part_instance":part_instances[0],"groups":[power]}));
    let after = s.snapshot().unwrap();
    let set = |project: &Project, key| Arc::clone(project.part_group_memberships.shared_set(key).unwrap());
    assert!(!Arc::ptr_eq(&set(&before, keys[0]), &set(&after, keys[0])));
    for key in &keys[1..] {
        assert!(Arc::ptr_eq(&set(&before, *key), &set(&after, *key)));
    }
    assert_eq!(after.groups_of_part_instance(keys[0]).collect::<Vec<_>>(), vec![power]);
    let held: BTreeSet<*const Uuid> = s
        .undo
        .iter()
        .filter_map(crate::HistoryEntry::project)
        .chain([&after])
        .flat_map(|project| keys.iter().filter_map(|key| project.part_group_memberships.shared_set(*key)).map(|set| set.as_ptr()))
        .collect();
    assert_eq!(held.len(), keys.len() + 1, "one set per membership edit across the whole history");
    apply(&mut s, "edit.undo", json!({}));
    let undone = s.snapshot().unwrap();
    for key in &keys {
        assert!(Arc::ptr_eq(&set(&before, *key), &set(&undone, *key)));
    }
    assert_eq!(memberships(&undone), memberships(&before));
}
