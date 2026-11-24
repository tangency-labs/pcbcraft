//! Part metadata travels through publication, pinning and offline storage unchanged.
use super::*;
use crate::library::parts::{METADATA_PART_FORMAT, PartDefinition};

const DESCRIPTION: &str = "Cryogenic reference resistor";
const KEYWORDS: [&str; 2] = ["Precision", "cryostat"];
const LINK_URL: &str = "https://example.invalid/ds.pdf";
const PART_PATH: &str = "library/parts/parts-demo/resistor.r1.json";

/// Canonical native documents carrying format-5 metadata, as a publisher would produce.
fn metadata_documents(description: &str) -> sources::Documents {
    let mut records = super::documents();
    let mut part: Json = serde_json::from_str(&records["parts/resistor.json"]).unwrap();
    part["format"] = json!(METADATA_PART_FORMAT);
    part["description"] = json!(description);
    part["keywords"] = json!(KEYWORDS);
    part["links"] = json!([{"label":"Datasheet","url":LINK_URL}]);
    records.insert("parts/resistor.json".into(), part.to_string());
    records
}

/// Load the exact source through the ordinary read command; a problem would hide the metadata.
fn load(session: &mut Session, documents: &sources::Documents) {
    let list = session.execute(sources::LOAD, &json!({"source":"native-parts","documents":documents})).unwrap();
    let loaded = list.as_array().unwrap().iter().find(|source| source["source"] == "native-parts").unwrap();
    assert_eq!(loaded["problems"], json!([]));
}

/// Place a description-tagged part using the resolved digest from its loaded source.
fn place(session: &mut Session) {
    let params = placement(session, false);
    apply(session, PLACE_PART, params);
}

/// A published /5 record pins its metadata, survives source removal and reopens from folders and archives.
#[test]
fn metadata_part_pins_description_keywords_and_links_offline() {
    let records = metadata_documents(DESCRIPTION);
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    load(&mut session, &records);
    place(&mut session);

    let reference = Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
    let resolved = session.libraries.resolve_part(&reference).unwrap();
    let digest = sources::part_digest(resolved);
    let definition = resolved.definition.as_ref().clone();
    assert_eq!(definition.format, METADATA_PART_FORMAT);
    assert_eq!(definition.description.as_deref(), Some(DESCRIPTION));
    assert_eq!(definition.keywords, KEYWORDS);
    assert_eq!(definition.links[0].url, LINK_URL);

    session.execute(sources::REMOVE, &json!({"source":"native-parts"})).unwrap();
    let snapshot = session.snapshot().unwrap();
    let pinned = snapshot.components().next().unwrap().part.as_ref().unwrap().resolved.definition.as_ref().clone();
    assert_eq!(pinned, definition);
    assert_eq!(pinned.search_text(), "cryogenic reference resistor precision cryostat");
    assert!(!pinned.search_text().contains("datasheet"));

    let native = encode_documents(snapshot.clone()).unwrap();
    let packed = encode_packed(snapshot).unwrap();
    let folder = decode_documents(&native).unwrap();
    let archive = decode_packed(&packed).unwrap();
    for reopened in [&folder, &archive.project] {
        let part = reopened.components().next().unwrap().part.as_ref().unwrap();
        assert_eq!(part.resolved.definition.as_ref(), &definition);
        assert_eq!(part.resolved.definition.description.as_deref(), Some(DESCRIPTION));
        assert_eq!(sources::part_digest(&part.resolved), digest);
    }
    assert_eq!(archive.documents, native);
    assert_eq!(encode_documents(folder).unwrap(), native);

    // The pinned native bytes are the part's own canonical encoding, including all metadata.
    let bytes = &native[PART_PATH];
    let text = std::str::from_utf8(bytes).unwrap();
    let record: Json = serde_json::from_str(text).unwrap();
    assert_eq!(record["format"], METADATA_PART_FORMAT);
    assert_eq!(record["description"], json!(DESCRIPTION));
    assert_eq!(record["keywords"], json!(KEYWORDS));
    assert_eq!(record["links"][0]["url"], json!(LINK_URL));
    assert_eq!(PartDefinition::read(text).unwrap().encode().unwrap().as_bytes(), bytes.as_slice());
}

/// Editing metadata at the same identity and revision changes the dependency digest and refuses
/// a pending placement whose capture still carries the old digest.
#[test]
fn metadata_edits_change_the_pinned_digest_and_reject_stale_placement() {
    let records = metadata_documents(DESCRIPTION);
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    load(&mut session, &records);
    let reference = Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
    let old = sources::part_digest(session.libraries.resolve_part(&reference).unwrap());
    let stale = placement(&session, false);

    let changed = metadata_documents("A different cryogenic description");
    session.execute(sources::REMOVE, &json!({"source":"native-parts"})).unwrap();
    load(&mut session, &changed);
    let new = sources::part_digest(session.libraries.resolve_part(&reference).unwrap());
    assert_ne!(old, new, "authored metadata is part of the pinned closure");
    let before = session.snapshot().unwrap();
    let mut captured = stale;
    captured["expected_revision"] = json!(session.revision());
    assert!(session.apply(PLACE_PART, &captured).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));

    let valid = placement(&session, false);
    assert_ne!(valid["definition_digest"], json!(old));
    apply(&mut session, PLACE_PART, valid);
    let pinned = session.snapshot().unwrap().components().next().unwrap().part.as_ref().unwrap().resolved.definition.clone();
    assert_eq!(pinned.description.as_deref(), Some("A different cryogenic description"));
}

/// Strict readers reject malformed or misowned metadata in a pinned record, not just in a source file.
#[test]
fn pinned_metadata_records_refuse_null_empty_oversized_and_malformed_links() {
    let records = metadata_documents(DESCRIPTION);
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    load(&mut session, &records);
    place(&mut session);
    let native = encode_documents(session.snapshot().unwrap()).unwrap();
    let pinned: Json = serde_json::from_slice(&native[PART_PATH]).unwrap();

    for (field, value) in [
        ("description", json!(null)),
        ("description", json!("")),
        ("description", json!("x".repeat(513))),
        ("description", json!("line\nbreak")),
        ("keywords", json!(null)),
        ("keywords", json!([])),
        ("keywords", json!(["Precision", " precision "])),
        ("keywords", json!(vec!["k"; 17])),
        ("links", json!(null)),
        ("links", json!([])),
        ("links", json!([{"label":"Datasheet","url":"ftp://example.invalid/ds.pdf"}])),
        ("links", json!([{"label":"Datasheet","url":"https://example.invalid/a b"}])),
        ("format", json!("pcbcraft-part/2")),
    ] {
        let mut record = pinned.clone();
        record[field] = value.clone();
        let mut changed = native.clone();
        changed.insert(PART_PATH.into(), serde_json::to_vec(&record).unwrap());
        assert!(decode_documents(&changed).is_err(), "{field}={value}");
    }

    // A valid /5 pinned record is unchanged by a decode/encode cycle.
    assert_eq!(encode_documents(decode_documents(&native).unwrap()).unwrap(), native);
}
