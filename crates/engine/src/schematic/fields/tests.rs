//! Field content and layout transactions retain geometry, history and native document ownership.
use super::*;
use crate::{
    PLACE_SYMBOL, SET_SYMBOL_FIELDS, Session,
    persistence::{ENTRY_POINT, decode_documents, encode_documents},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Two independent instances share a definition, while their properties remain independent.
fn fixture() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    for x in [0, 25_400_000] {
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    s
}

/// Metadata and label layout alter only their owning documents, and undo restores exact snapshots.
#[test]
fn metadata_and_appearance_are_atomic_persistent_and_instance_owned() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let baseline = encode_documents(before.clone()).unwrap();
    let mut metadata = Metadata::default();
    metadata.fields.insert("manufacturer".into(), Field { name: "Manufacturer".into(), value: "Example".into() });
    metadata.fields.insert(new_id().unwrap(), Field { name: "Supplier code".into(), value: "ABC123".into() });
    metadata.attributes.do_not_populate = true;
    let mut params = json!({"symbol":"symbol-2","reference":"R1","value":"4.7k","metadata":metadata,"expected_revision":s.revision()});
    s.apply(SET_SYMBOL_FIELDS, &params).unwrap();
    let with_metadata = s.snapshot().unwrap();
    assert_eq!(with_metadata.components().nth(1).unwrap().value(), "R");
    assert!(with_metadata.components().nth(1).unwrap().metadata.is_empty());
    assert!(Arc::ptr_eq(&before.symbols()[0].definition, &with_metadata.symbols()[0].definition));
    let first = encode_documents(with_metadata.clone()).unwrap();
    assert_eq!(first.iter().filter(|(path, bytes)| baseline.get(*path) != Some(*bytes)).map(|(p, _)| p.as_str()).collect::<Vec<_>>(), [ENTRY_POINT]);
    let layouts = Layouts::from([(
        "reference".into(),
        Layout { offset: Some(Point { x_nm: 2_540_000, y_nm: -5_080_000 }), rotation: 90, ..Layout::default() },
    )]);
    params = json!({"symbol":"symbol-2","reference":"R1","value":"4.7k","field_layout":layouts,"expected_revision":s.revision()});
    let revision = s.revision();
    s.apply(SET_SYMBOL_FIELDS, &params).unwrap();
    assert_eq!(s.revision(), revision + 1);
    let after = s.snapshot().unwrap();
    assert_eq!(after.symbols()[0].position, before.symbols()[0].position);
    assert_eq!(after.symbols()[0].pins(), before.symbols()[0].pins());
    assert!(before.same_connectivity(&after));
    let documents = encode_documents(after.clone()).unwrap();
    assert_eq!(
        documents.iter().filter(|(path, bytes)| first.get(*path) != Some(*bytes)).map(|(p, _)| p.clone()).collect::<Vec<_>>(),
        [format!("sheets/{}.json", after.sheet_target().sheet)]
    );
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    assert!(crate::persistence::encode(after.clone()).is_err(), "legacy export must not discard fields");
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&with_metadata, &s.snapshot().unwrap()));
    s.apply("edit.redo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&after, &s.snapshot().unwrap()));
    params["expected_revision"] = json!(s.revision());
    let revision = s.revision();
    s.apply(SET_SYMBOL_FIELDS, &params).unwrap();
    assert_eq!(s.revision(), revision, "unchanged property apply is a no-op");
}

/// Invalid content/layout rejects all accompanying edits, as does stale revision ownership.
#[test]
fn invalid_fields_never_partially_edit_content_or_geometry() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let mut metadata = Metadata::default();
    metadata.fields.insert("manufacturer".into(), Field { name: "Reference".into(), value: "bad".into() });
    for extra in [
        json!({"metadata":metadata}),
        json!({"metadata":null}),
        json!({"field_layout":{"missing":Layout::default()}}),
        json!({"field_layout":{"reference":Layout{height_nm:-1,..Layout::default()}}}),
        json!({"field_layout":{"reference":Layout{offset:Some(Point{x_nm:i64::MIN,y_nm:0}),..Layout::default()}}}),
    ] {
        let mut params = json!({"symbol":"symbol-2","reference":"R7","value":"10k","x_nm":12_700_000,"expected_revision":s.revision()});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(s.apply(SET_SYMBOL_FIELDS, &params).is_err(), "{extra}");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    assert!(
        s.apply(SET_SYMBOL_FIELDS, &json!({"symbol":"symbol-2","reference":"R7","value":"10k","metadata":Metadata::default(),"expected_revision":0}))
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// New generic parts avoid assumed ratings; older pinned library revisions keep authored defaults.
#[test]
fn neutral_generic_defaults_preserve_older_revisions() {
    for (id, value, old) in [("resistor", "R", "1k"), ("capacitor", "C", "100nF")] {
        let library = crate::library::starter();
        assert_eq!(library.symbol(id).unwrap().default_value(), value);
        for revision in [1, 2] {
            assert_eq!(library.symbol_revision(id, revision).unwrap().default_value(), old);
        }
        assert_eq!(library.symbol(id).unwrap().pins(), library.symbol_revision(id, 2).unwrap().pins());
    }
}

/// Versions gate the new content so old vocabulary cannot silently acquire newer fields.
#[test]
fn native_versions_reject_downgraded_fields_and_null_extensions() {
    let mut s = fixture();
    let mut metadata = Metadata::default();
    metadata.attributes.exclude_from_bom = true;
    s.apply(
        SET_SYMBOL_FIELDS,
        &json!({"symbol":"symbol-2","reference":"R1","value":"R","metadata":metadata,
        "field_layout":{"value":Layout{visible:false,..Layout::default()}},"expected_revision":s.revision()}),
    )
    .unwrap();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let sheet = format!("sheets/{}.json", s.project().unwrap().sheet_target().sheet);
    for (path, key, value) in [(ENTRY_POINT, "format", json!("pcbcraft-project/6")), (sheet.as_str(), "format", json!("pcbcraft-sheet/3"))] {
        let mut docs = original.clone();
        let mut record: Value = serde_json::from_slice(&docs[path]).unwrap();
        record[key] = value;
        docs.insert(path.into(), serde_json::to_vec(&record).unwrap());
        assert!(decode_documents(&docs).is_err());
    }
}

/// Board exclusion prevents placement, and cannot silently remove an existing board footprint.
#[test]
fn board_exclusion_is_enforced_without_destroying_placements() {
    let mut s = fixture();
    let mut metadata = Metadata::default();
    metadata.attributes.exclude_from_board = true;
    let fields = json!({"symbol":"symbol-2","reference":"R1","value":"R","metadata":metadata,"expected_revision":s.revision()});
    s.apply(SET_SYMBOL_FIELDS, &fields).unwrap();
    let before = s.snapshot().unwrap();
    let placement =
        json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":s.revision()});
    assert!(s.apply(crate::PLACE_FOOTPRINT, &placement).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply("edit.undo", &Value::Null).unwrap();
    let mut placement = placement;
    placement["expected_revision"] = json!(s.revision());
    s.apply(crate::PLACE_FOOTPRINT, &placement).unwrap();
    let placed = s.snapshot().unwrap();
    let mut fields = fields;
    fields["expected_revision"] = json!(s.revision());
    assert!(s.apply(SET_SYMBOL_FIELDS, &fields).is_err());
    assert!(Arc::ptr_eq(&placed, &s.snapshot().unwrap()));
}
