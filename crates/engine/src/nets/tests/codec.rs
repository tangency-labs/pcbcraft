use super::*;
use crate::persistence::Documents;

/// Exhaustive malformed records keep negative cases from being skipped by string typos.
#[derive(Clone, Copy, Debug)]
enum BrokenRegistry {
    NilId,
    NilProject,
    ForeignScope,
    MissingPrimary,
    UnknownKind,
    WrongTerminalForm,
    DuplicateFallback,
    FourFallbacks,
    DuplicateRecord,
    UnknownRecord,
    UnknownRegistry,
    UnknownState,
    UnknownActiveField,
    Empty,
    Excessive,
}

/// Exact historical-source fields exercised independently by the negative codec cases.
#[derive(Clone, Copy, Debug)]
enum BrokenSource {
    Capture,
    Digest,
    Identity,
    MissingIdentity,
    Owner,
}

/// Produce both storage homes through the shared promotion command and production encoder.
fn fixture(board: bool) -> (Documents, String, Value) {
    let (mut session, pins) = circuit(2, board);
    connect(&mut session, &pins[0], &pins[1]);
    promote(&mut session, &pins[0]);
    let project = session.snapshot().unwrap();
    let path = project.instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
    let documents = encode_documents(project).unwrap();
    let value = serde_json::from_slice(&documents[&path]).unwrap();
    (documents, path, value)
}

/// Find the registry object in either schema, without changing its sole storage home.
fn registry(value: &mut Value) -> &mut Value {
    if value.get("schematic_instances").is_some() { &mut value["schematic_instances"]["net_registry"] } else { &mut value["net_registry"] }
}

/// Reject malformed, foreign or oversized records instead of repairing persisted identity.
#[test]
fn registry_codec_refuses_invalid_authority_and_preserves_the_input_snapshot() {
    for board in [false, true] {
        let (docs, path, original) = fixture(board);
        for case in [
            BrokenRegistry::NilId,
            BrokenRegistry::NilProject,
            BrokenRegistry::ForeignScope,
            BrokenRegistry::MissingPrimary,
            BrokenRegistry::UnknownKind,
            BrokenRegistry::WrongTerminalForm,
            BrokenRegistry::DuplicateFallback,
            BrokenRegistry::FourFallbacks,
            BrokenRegistry::DuplicateRecord,
            BrokenRegistry::UnknownRecord,
            BrokenRegistry::UnknownRegistry,
            BrokenRegistry::UnknownState,
            BrokenRegistry::UnknownActiveField,
            BrokenRegistry::Empty,
            BrokenRegistry::Excessive,
        ] {
            let mut value = original.clone();
            let registry = registry(&mut value);
            let primary = registry["records"][0]["primary"].clone();
            match case {
                BrokenRegistry::NilId => registry["records"][0]["id"] = json!(Uuid::nil()),
                BrokenRegistry::NilProject => registry["records"][0]["primary"]["project"] = json!(Uuid::nil()),
                BrokenRegistry::ForeignScope => registry["records"][0]["primary"]["scope"]["id"] = json!(Uuid::from_u128(11)),
                BrokenRegistry::MissingPrimary => registry["records"][0]["primary"]["object"] = json!(999_999),
                BrokenRegistry::UnknownKind => registry["records"][0]["primary"]["kind"] = json!("future_kind"),
                BrokenRegistry::WrongTerminalForm => registry["records"][0]["primary"]["occurrence"] = json!(Uuid::from_u128(1)),
                BrokenRegistry::DuplicateFallback => registry["records"][0]["fallbacks"] = json!([primary]),
                BrokenRegistry::FourFallbacks => {
                    registry["records"][0]["fallbacks"] = json!([primary.clone(), primary.clone(), primary.clone(), primary])
                }
                BrokenRegistry::DuplicateRecord => {
                    let record = registry["records"][0].clone();
                    registry["records"].as_array_mut().unwrap().push(record);
                }
                BrokenRegistry::UnknownRecord => registry["records"][0]["unknown"] = json!(true),
                BrokenRegistry::UnknownRegistry => registry["unknown"] = json!(true),
                BrokenRegistry::UnknownState => registry["records"][0]["state"] = json!({"kind":"future_state"}),
                BrokenRegistry::UnknownActiveField => registry["records"][0]["state"]["unknown"] = json!(true),
                BrokenRegistry::Empty => registry["records"] = json!([]),
                BrokenRegistry::Excessive => registry["records"] = json!(vec![registry["records"][0].clone(); MAX_RECORDS + 1]),
            }
            let mut bad = docs.clone();
            bad.insert(path.clone(), serde_json::to_vec(&value).unwrap());
            assert!(decode_documents(&bad).is_err(), "accepted {case:?} in {path}");
        }
        assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
        let mut encoder = crate::persistence::DocumentsEncoder::new(decode_documents(&docs).unwrap());
        let incremental = loop {
            if let Some(documents) = encoder.step(1).unwrap() {
                break documents;
            }
        };
        assert_eq!(incremental, docs);
    }
}

/// Project/16 cannot carry registry data, project/17 cannot omit it, and board instance headers
/// must select instances/2. Explicit null never serves as an older-vocabulary omission.
#[test]
fn registry_vocabulary_is_versioned_and_has_exactly_one_home() {
    for board in [false, true] {
        let (docs, path, original) = fixture(board);
        let mut downgraded = docs.clone();
        let mut entry: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
        entry["format"] = json!("pcbcraft-project/16");
        downgraded.insert(ENTRY_POINT.into(), serde_json::to_vec(&entry).unwrap());
        if board {
            let mut instance = original.clone();
            instance["format"] = json!("pcbcraft-instances/1");
            downgraded.insert(path.clone(), serde_json::to_vec(&instance).unwrap());
        }
        assert!(decode_documents(&downgraded).unwrap_err().contains("Net registry records require project format 17"));
        for absent in [true, false] {
            let mut missing = docs.clone();
            let mut value = original.clone();
            if absent {
                if board {
                    value.as_object_mut().unwrap().remove("net_registry");
                } else {
                    value["schematic_instances"].as_object_mut().unwrap().remove("net_registry");
                }
            } else {
                *registry(&mut value) = Value::Null;
            }
            missing.insert(path.clone(), serde_json::to_vec(&value).unwrap());
            assert!(decode_documents(&missing).is_err());
        }
        if board {
            let mut bad = docs.clone();
            let mut value = original.clone();
            value["format"] = json!("pcbcraft-instances/1");
            bad.insert(path.clone(), serde_json::to_vec(&value).unwrap());
            assert!(decode_documents(&bad).is_err());
            let mut double = docs.clone();
            entry = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
            entry["schematic_instances"] = json!({"circuit_uses":original["circuit_uses"],"sheet_occurrences":original["sheet_occurrences"],"net_registry":original["net_registry"]});
            double.insert(ENTRY_POINT.into(), serde_json::to_vec(&entry).unwrap());
            assert!(decode_documents(&double).is_err());
        }
    }
}

/// A historical source remains a typed capture, not a current network membership. Missing
/// nullable source identities, invalid captures and foreign/unknown source IDs are refused.
#[test]
fn lineage_requires_exact_snapshot_and_identity_fields() {
    let (mut session, pins) = circuit(4, false);
    connect(&mut session, &pins[0], &pins[1]);
    let middle = connect(&mut session, &pins[1], &pins[2]);
    connect(&mut session, &pins[2], &pins[3]);
    promote(&mut session, &pins[0]);
    apply(&mut session, crate::DELETE_WIRE, json!({"wire":middle}));
    let docs = encode_documents(session.snapshot().unwrap()).unwrap();
    let original: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
    for case in [BrokenSource::Capture, BrokenSource::Digest, BrokenSource::Identity, BrokenSource::MissingIdentity, BrokenSource::Owner] {
        let mut value = original.clone();
        let records = registry(&mut value)["records"].as_array_mut().unwrap();
        let source = &mut records.iter_mut().find(|record| !record["lineage"].as_array().unwrap().is_empty()).unwrap()["lineage"][0];
        match case {
            BrokenSource::Capture => source["snapshot"]["capture_id"] = json!(Uuid::nil()),
            BrokenSource::Digest => source["snapshot"]["content_digest"] = json!("not-a-digest"),
            BrokenSource::Identity => source["net"] = json!(Uuid::from_u128(11)),
            BrokenSource::MissingIdentity => {
                source.as_object_mut().unwrap().remove("net");
            }
            BrokenSource::Owner => source["anchor"]["project"] = json!(Uuid::from_u128(11)),
        }
        let mut bad = docs.clone();
        bad.insert(ENTRY_POINT.into(), serde_json::to_vec(&value).unwrap());
        assert!(decode_documents(&bad).is_err(), "{case:?}");
    }
}

/// Deleted anchors still reserve their numeric object IDs through the project watermark;
/// a forged lower watermark cannot authorize their later reuse after reopening.
#[test]
fn historical_anchor_ids_remain_covered_by_the_numeric_watermark() {
    let (mut session, pins) = circuit(2, false);
    connect(&mut session, &pins[0], &pins[1]);
    promote(&mut session, &pins[1]);
    let selected = Selection { symbols: pins.iter().map(|pin| pin["symbol"].as_str().unwrap().into()).collect(), ..Selection::default() };
    apply(&mut session, crate::DELETE_ITEMS, json!({"items":selected}));
    assert!(session.project().unwrap().components().next().is_none());
    let mut docs = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut entry: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
    entry["numeric_id_high_water"] = json!(2);
    docs.insert(ENTRY_POINT.into(), serde_json::to_vec(&entry).unwrap());
    assert!(decode_documents(&docs).unwrap_err().contains("watermark"));
}
