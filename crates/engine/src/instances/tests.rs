use super::*;
use crate::{
    Session,
    persistence::{ENTRY_POINT, decode, decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::{Value, json};

/// Retain a production-encoded project/14 snapshot to exercise migration from the current layout.
fn legacy_folder() -> Arc<Project> {
    let p = decode(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap();
    Arc::new(p.with_board_folders(".").unwrap())
}

/// Apply a shared revision-checked command, naming sheet and board targets in each test.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    if !matches!(command, "project.new" | "edit.undo" | "edit.redo") {
        params["expected_revision"] = json!(s.revision());
    }
    s.apply(command, &params).unwrap();
}

/// Fixed byte-encoding vectors protect compatibility across platforms and independent opens.
#[test]
fn legacy_uuid_vectors_are_domain_separated_and_stable() {
    let p = Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap();
    let s = Uuid::parse_str("10213243-5465-7687-98a9-bacbdcedfe0f").unwrap();
    assert_eq!(legacy_id(b"pcbcraft:legacy-circuit-use:1\0", p, None).to_string(), "3a62a9eb-47a7-876f-ac20-c8a4fc611074");
    assert_eq!(legacy_id(b"pcbcraft:legacy-sheet-occurrence:1\0", p, Some(s)).to_string(), "41ef7bfe-b5ae-8afe-beed-f319542cf74d");
    assert_ne!(legacy_id(b"pcbcraft:legacy-circuit-use:1\0", p, None), legacy_id(b"pcbcraft:legacy-sheet-occurrence:1\0", p, Some(s)));
    assert_ne!(legacy_id(b"pcbcraft:legacy-sheet-occurrence:1\0", p, Some(s)), legacy_id(b"pcbcraft:legacy-sheet-occurrence:1\0", s, Some(p)));
}

/// Project/14 remains unchanged through Open; migration persists the same identities without
/// moving board folders or rewriting physical, schematic or pinned-definition payloads.
#[test]
fn current_folder_migration_preserves_payloads_and_inspected_legacy_ids() {
    let before = encode_documents(legacy_folder()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&before[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/14");
    let a = decode_documents(&before).unwrap();
    let b = decode_documents(&before).unwrap();
    assert_eq!(a.sheet_instances(), b.sheet_instances());
    assert!(!a.has_sheet_occurrences());
    assert_eq!(encode_documents(a.clone()).unwrap(), before);
    let migrated = Arc::new(a.with_sheet_occurrences().unwrap());
    assert_eq!(migrated.sheet_instances(), a.sheet_instances());
    let after = encode_documents(migrated.clone()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&after[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/16");
    assert_eq!(migrated.board_document_path(), a.board_document_path());
    for (path, bytes) in &before {
        if path != ENTRY_POINT {
            assert_eq!(&after[path], bytes, "{path}");
        }
    }
    let instances = migrated.instances_document_path().unwrap();
    assert!(instances.ends_with("/instances.json"));
    assert_eq!(after.len(), before.len() + 1);
    let reopened = decode_documents(&after).unwrap();
    assert_eq!(reopened.sheet_instances(), a.sheet_instances());
    assert_eq!(encode_documents(reopened).unwrap(), after);
    let mut encoder = crate::persistence::DocumentsEncoder::new(migrated.clone());
    let incremental = loop {
        if let Some(documents) = encoder.step(1).unwrap() {
            break documents;
        }
    };
    assert_eq!(incremental, after, "frame-sized encoding preserves the same instance document");
    let packed = encode_packed(migrated).unwrap();
    let unpacked = decode_packed(&packed).unwrap();
    assert_eq!(unpacked.documents, after);
}

/// New projects allocate independent identities; creating the first board changes only their
/// storage home, and undo/redo restores that home and the exact same logical instances.
#[test]
fn new_instances_follow_explicit_first_board_creation_and_history() {
    let mut s = Session::default();
    apply(&mut s, "project.new", json!({}));
    let p = s.project().unwrap();
    let records = p.sheet_instances();
    assert!(p.has_sheet_occurrences());
    assert_ne!(records.circuit_uses[0].id, legacy_id(b"pcbcraft:legacy-circuit-use:1\0", p.id, None));
    assert_ne!(records.sheet_occurrences[0].id, legacy_id(b"pcbcraft:legacy-sheet-occurrence:1\0", p.id, Some(p.sheet_id)));
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&before[ENTRY_POINT]).unwrap();
    assert_eq!(entry["schematic_instances"], serde_json::to_value(&records).unwrap());
    assert_eq!(before.len(), 2, "entry point and one sheet, with no physical files");
    apply(&mut s, crate::board::identity::CREATE_BOARD, json!({}));
    let p = s.project().unwrap();
    assert_eq!(p.sheet_instances(), records);
    let path = p.instances_document_path().unwrap();
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&after[ENTRY_POINT]).unwrap();
    assert!(entry.get("schematic_instances").is_none());
    assert_eq!(serde_json::from_slice::<Value>(&after[&path]).unwrap()["board"], json!(p.board_target().unwrap().board));
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    apply(&mut s, "edit.redo", json!({}));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
}

/// Nested sheet relationships track authored parent slots without changing occurrence IDs.
/// Detach/reparent, rename and undo never derive a new identity from the new hierarchy path.
#[test]
fn hierarchical_occurrences_keep_ids_through_reparenting_and_rename() {
    let mut s = Session::default();
    apply(&mut s, "project.new", json!({}));
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, crate::hierarchy::CREATE_CHILD, json!({"sheet":main.id(),"name":"Child","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, crate::hierarchy::CREATE_CHILD, json!({"sheet":child.id(),"name":"Nested","x_nm":0,"y_nm":0}));
    let nested = s.project().unwrap().sheets().nth(2).unwrap().target;
    let before = s.project().unwrap().sheet_occurrence(nested).unwrap();
    let parent = s.project().unwrap().sheet_occurrence(child).unwrap();
    let slot = s.project().unwrap().for_sheet(child).unwrap().subsheets[0].id;
    assert_eq!(before.parent, Some(parent.id));
    assert_eq!(before.parent_slot, Some(slot));
    apply(&mut s, crate::hierarchy::REMOVE_SHEET, json!({"sheet":child.id(),"id":format!("sheet-symbol-{slot}")}));
    apply(&mut s, crate::hierarchy::PLACE_SHEET, json!({"sheet":main.id(),"child":nested.id(),"x_nm":20_000_000,"y_nm":0}));
    apply(&mut s, crate::sheets::RENAME_SHEET, json!({"sheet":nested.id(),"name":"Renamed"}));
    let after = s.project().unwrap().sheet_occurrence(nested).unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(after.circuit_use, before.circuit_use);
    assert_eq!(after.parent, Some(s.project().unwrap().sheet_occurrence(main).unwrap().id));
    assert_ne!(after.parent_slot, before.parent_slot);
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(decode_documents(&docs).unwrap().sheet_instances(), s.project().unwrap().sheet_instances());
}

/// Incompatible, malformed and extra records are rejected before an existing session changes.
#[test]
fn instance_codec_refuses_foreign_owners_duplicates_and_invalid_relationships() {
    let valid = Arc::new(legacy_folder().with_sheet_occurrences().unwrap());
    let docs = encode_documents(valid.clone()).unwrap();
    let path = valid.instances_document_path().unwrap();
    let original: Value = serde_json::from_slice(&docs[&path]).unwrap();
    let mut cases = Vec::new();
    for pointer in [
        "/board",
        "/circuit_uses/0/id",
        "/sheet_occurrences/0/id",
        "/sheet_occurrences/0/definition",
        "/sheet_occurrences/0/circuit_use",
        "/sheet_occurrences/0/parent",
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = json!(if pointer == "/sheet_occurrences/0/id" { valid.id } else { Uuid::from_u128(55) });
        cases.push(value);
    }
    let mut duplicate = original.clone();
    let record = duplicate["sheet_occurrences"][0].clone();
    duplicate["sheet_occurrences"].as_array_mut().unwrap().push(record);
    cases.push(duplicate);
    let mut unknown = original.clone();
    unknown["unknown"] = json!(true);
    cases.push(unknown);
    for pointer in ["/circuit_uses/0/id", "/sheet_occurrences/0/id", "/sheet_occurrences/0/circuit_use"] {
        let mut nil = original.clone();
        *nil.pointer_mut(pointer).unwrap() = json!(Uuid::nil());
        cases.push(nil);
    }
    for field in ["circuit_uses", "sheet_occurrences"] {
        let mut empty = original.clone();
        empty[field] = json!([]);
        cases.push(empty);
    }
    let mut repeated_use = original.clone();
    let circuit_use = repeated_use["circuit_uses"][0].clone();
    repeated_use["circuit_uses"].as_array_mut().unwrap().push(circuit_use);
    cases.push(repeated_use);
    let mut unknown_occurrence = original.clone();
    unknown_occurrence["sheet_occurrences"][0]["unknown"] = json!(true);
    cases.push(unknown_occurrence);
    let mut slot = original.clone();
    slot["sheet_occurrences"][0]["parent_slot"] = json!(42);
    cases.push(slot);
    let mut missing = original.clone();
    missing["sheet_occurrences"][0].as_object_mut().unwrap().remove("parent");
    cases.push(missing);
    let mut version = original.clone();
    version["format"] = json!("pcbcraft-instances/99");
    cases.push(version);
    for case in cases {
        let mut bad = docs.clone();
        bad.insert(path.clone(), serde_json::to_vec(&case).unwrap());
        assert!(decode_documents(&bad).is_err(), "{case}");
    }
    let mut absent = docs.clone();
    absent.remove(&path);
    assert!(decode_documents(&absent).is_err());
    let mut double_home = docs.clone();
    let mut entry: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
    entry["schematic_instances"] = serde_json::to_value(valid.sheet_instances()).unwrap();
    double_home.insert(ENTRY_POINT.into(), serde_json::to_vec(&entry).unwrap());
    assert!(decode_documents(&double_home).is_err());
    assert!(crate::persistence::encode(valid).unwrap_err().contains("occurrence"));
}

/// Component and accepted physical identities remain shared across flat sheets; occurrence
/// migration cannot change references, values, maps, connectivity or reviewed update state.
#[test]
fn occurrence_migration_preserves_components_ratsnest_and_accepted_board_state() {
    let mut s = Session::default();
    s.open_snapshot(legacy_folder());
    let main = s.project().unwrap().sheet_target();
    let board = s.project().unwrap().board_target().unwrap().board;
    apply(&mut s, crate::sheets::CREATE_SHEET, json!({"name":"Other root"}));
    apply(&mut s, crate::INITIALIZE_BOARD_SETUP, json!({"board":board,"copper_layers":2,"resolve_existing_footprints":true}));
    let components: Vec<_> = s.project().unwrap().components().map(|component| component.id()).collect();
    for component in components {
        apply(
            &mut s,
            crate::ASSIGN_FOOTPRINT,
            json!({"component":component,"library":"starter","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"2","2":"1"}}),
        );
    }
    apply(&mut s, crate::board::update::INITIALIZE_BASELINE, json!({"board":board}));
    let old = s.snapshot().unwrap();
    let original = encode_documents(old.clone()).unwrap();
    let next = Arc::new(old.with_sheet_occurrences().unwrap());
    assert_eq!(
        next.components().map(|component| component.inspect()).collect::<Vec<_>>(),
        old.components().map(|component| component.inspect()).collect::<Vec<_>>()
    );
    assert_eq!(next.footprint_assignments(), old.footprint_assignments());
    assert_eq!(next.board.baseline, old.board.baseline);
    assert_eq!(next.boards().next().unwrap().ratsnest(), old.boards().next().unwrap().ratsnest());
    assert_eq!(next.for_sheet(main).unwrap().inspect_schematic(), old.for_sheet(main).unwrap().inspect_schematic());
    let records = next.sheet_instances();
    assert_eq!(records.circuit_uses.len(), 1);
    assert!(records.sheet_occurrences.iter().all(|occurrence| occurrence.circuit_use == records.circuit_uses[0].id));
    let docs = encode_documents(next).unwrap();
    for (path, bytes) in original.iter().filter(|(path, _)| path.as_str() != ENTRY_POINT) {
        assert_eq!(&docs[path], bytes);
    }
    assert_eq!(decode_documents(&docs).unwrap().sheet_instances(), records);
}

/// Migration of project/15 keeps two flat-sheet drawings in one native part, reference and
/// physical package, including pinned dependencies, maps, accepted baseline and connectivity.
#[test]
fn native_multiunit_part_keeps_one_package_across_occurrence_migration() {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({"create_board":true}));
    session
        .execute(
            "library.load_source",
            &json!({"source":"native-units","documents":{
                "library.json":include_str!("../../../../examples/native-parts/library.json"),
                "symbols/dual-amplifier.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json"),
                "parts/dual-amplifier.json":include_str!("../../../../examples/native-parts/parts/dual-amplifier.json"),
                "footprints/illustrative-six-pad.json":include_str!("../../../../examples/native-parts/footprints/illustrative-six-pad.json")
            }}),
        )
        .unwrap();
    let reference = crate::library::parts::Reference { library: "parts-demo".into(), id: "dual-amplifier".into(), revision: 1 };
    let resolved = session.libraries.resolve_part(&reference).unwrap();
    let digest = crate::library::sources::part_digest(resolved);
    apply(
        &mut session,
        crate::part_instances::PLACE_PART,
        json!({"library":"parts-demo","definition":"dual-amplifier","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    let component = session.project().unwrap().components().next().unwrap().id();
    apply(&mut session, crate::sheets::CREATE_SHEET, json!({"name":"Second root"}));
    let second = session.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut session, crate::PLACE_UNIT, json!({"sheet":second.id(),"component":component,"unit":"b","x_nm":0,"y_nm":0}));
    let board = session.project().unwrap().board_target().unwrap().board;
    apply(&mut session, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"board":board,"component":component,"x_nm":0,"y_nm":0}));
    apply(&mut session, crate::board::update::INITIALIZE_BASELINE, json!({"board":board}));
    let historical = crate::persistence::diagnostic_snapshot(session.snapshot().unwrap());
    let before = encode_documents(historical).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&before[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/15");
    let original = decode_documents(&before).unwrap();
    assert!(!original.has_sheet_occurrences());
    assert_eq!(encode_documents(original.clone()).unwrap(), before);
    let migrated = crate::persistence::migrate_board_layout(before.clone(), original.layout_root().unwrap()).unwrap();
    let reopened = decode_documents(&migrated).unwrap();
    assert_eq!(reopened.sheet_instances(), original.sheet_instances());
    assert_eq!(reopened.sheet_instances().circuit_uses.len(), 1);
    assert_eq!(reopened.sheet_instances().sheet_occurrences.len(), 2);
    assert_eq!(reopened.components().count(), 1);
    assert_eq!(reopened.all_symbols().count(), 2);
    assert!(reopened.all_symbols().all(|symbol| symbol.component_id() == component));
    assert_eq!(reopened.components().next().unwrap().inspect(), original.components().next().unwrap().inspect());
    assert_eq!(reopened.footprint_assignments(), original.footprint_assignments());
    assert_eq!(reopened.board.baseline, original.board.baseline);
    assert_eq!(reopened.boards().next().unwrap().ratsnest(), original.boards().next().unwrap().ratsnest());
    for (path, bytes) in before.iter().filter(|(path, _)| path.as_str() != ENTRY_POINT) {
        assert_eq!(&migrated[path], bytes, "{path}");
    }
    assert_eq!(encode_documents(reopened.clone()).unwrap(), migrated);
    assert_eq!(decode_packed(&encode_packed(reopened).unwrap()).unwrap().documents, migrated);
}
