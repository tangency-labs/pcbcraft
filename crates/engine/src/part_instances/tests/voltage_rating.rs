use super::*;
use crate::{
    authoring::publishing::new_library,
    library::parts::{CATEGORY_FORMAT, Category, CategoryBinding, Field, FieldType, Package, Quantity},
    part_authoring::Record as Draft,
};

const LIBRARY: &str = "rating-test";
const LICENSE: &str = "MIT OR Apache-2.0";
const VOLTAGE_RATING_FIELD: &str = "fixture:voltage-rating";

/// This category belongs to the synthetic fixture; shared drawing definitions carry no ratings.
fn rating_category() -> Category {
    Category {
        format: CATEGORY_FORMAT.into(),
        library: LIBRARY.into(),
        id: "capacitor".into(),
        revision: 1,
        name: "Synthetic capacitor".into(),
        license: LICENSE.into(),
        fields: BTreeMap::from([(
            VOLTAGE_RATING_FIELD.into(),
            Field { name: "Voltage rating".into(), value_type: FieldType::Quantity { quantity: Quantity::Voltage, unit: "V".into() } },
        )]),
    }
}

/// Feed production-encoded native records to the same command that loads other library sources.
fn load_rating_source(session: &mut Session, documents: &sources::Documents) {
    let sources = session.execute(sources::LOAD, &json!({"source":LIBRARY,"documents":documents})).unwrap();
    let source = sources.as_array().unwrap().iter().find(|source| source["source"] == LIBRARY).unwrap();
    assert_eq!(source["problems"], json!([]));
}

/// Generate new immutable part bytes through the shared publication preparation command.
fn prepare(session: &mut Session, record: &Draft, documents: &sources::Documents) -> Result<Json, crate::CommandError> {
    session.execute(sources::PREPARE_PART_PUBLICATION, &json!({"record":record,"documents":documents}))
}

/// Shared commands refuse amps by field name and preserve exact optional ratings in offline saves.
#[test]
fn voltage_rating_round_trips_in_generic_and_manufacturer_parts() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    let category = rating_category();
    let mut documents = sources::Documents::from([
        ("library.json".into(), new_library(LIBRARY, "Synthetic rating test", LICENSE).unwrap()),
        ("categories/capacitor.json".into(), category.encode().unwrap()),
    ]);
    load_rating_source(&mut session, &documents);
    let field = &category.fields[VOLTAGE_RATING_FIELD];
    let rating = field.parse_input("50 V").unwrap();
    let number = json!({"type":"quantity","value":"50","unit":"V"});
    assert_eq!(json!(rating), number);
    assert_eq!(field.parse_input("50000 mV").unwrap(), rating);
    assert_eq!(field.parse_input("50 A").unwrap_err(), "Expected V or a compatible SI-prefixed unit.");
    let amps = Value::Quantity { value: "50".into(), unit: "A".into() };
    let milliamps = Value::Quantity { value: "50".into(), unit: "mA".into() };

    let mut unrated = Draft::new_part(Reference { library: "basic-components".into(), id: "capacitor".into(), revision: 3 }).unwrap();
    let Draft::Part(part) = &mut unrated else { panic!("expected generic draft") };
    part.id = "unrated".into();
    part.name = "Synthetic capacitor without a rating".into();
    part.category = Some(CategoryBinding { definition: category.reference(), primary_value: Some(VOLTAGE_RATING_FIELD.into()) });
    part.packages.insert(
        "test".into(),
        Package {
            name: "Synthetic test package".into(),
            footprint: Reference { library: "basic-components".into(), id: "capacitor-0603".into(), revision: 4 },
            mapping: BTreeMap::from([("1".into(), vec!["1".into()]), ("2".into(), vec!["2".into()])]),
            unconnected_pads: Vec::new(),
        },
    );
    let mut rated = unrated.clone();
    let Draft::Part(part) = &mut rated else { panic!("expected generic draft") };
    part.id = "rated".into();
    part.name = "Synthetic rated capacitor".into();
    part.fixed_parameters.insert(VOLTAGE_RATING_FIELD.into(), amps.clone());
    assert_voltage_unit_error(prepare(&mut session, &rated, &documents).unwrap_err());
    let Draft::Part(part) = &mut rated else { panic!("expected generic draft") };
    part.fixed_parameters.insert(VOLTAGE_RATING_FIELD.into(), milliamps.clone());
    assert_rating_error(prepare(&mut session, &rated, &documents).unwrap_err(), "mA");
    let Draft::Part(part) = &mut rated else { panic!("expected generic draft") };
    part.fixed_parameters.insert(VOLTAGE_RATING_FIELD.into(), rating.clone());
    for draft in [&unrated, &rated] {
        let prepared = prepare(&mut session, draft, &documents).unwrap();
        documents.extend(serde_json::from_value::<sources::Documents>(prepared["documents"].clone()).unwrap());
    }
    load_rating_source(&mut session, &documents);

    let reference = Reference { library: LIBRARY.into(), id: "unrated".into(), revision: 1 };
    let mut orderable = Draft::new_orderable(session.libraries(), reference).unwrap();
    let Draft::Orderable(part) = &mut orderable else { panic!("expected manufacturer draft") };
    part.id = "test-mpn".into();
    part.manufacturer = "Synthetic test manufacturer".into();
    part.mpn = "TEST-CAP".into();
    part.specifications.insert(VOLTAGE_RATING_FIELD.into(), amps);
    assert_voltage_unit_error(prepare(&mut session, &orderable, &documents).unwrap_err());
    let Draft::Orderable(part) = &mut orderable else { panic!("expected manufacturer draft") };
    part.specifications.insert(VOLTAGE_RATING_FIELD.into(), milliamps);
    assert_rating_error(prepare(&mut session, &orderable, &documents).unwrap_err(), "mA");
    let Draft::Orderable(part) = &mut orderable else { panic!("expected manufacturer draft") };
    part.specifications.insert(VOLTAGE_RATING_FIELD.into(), rating.clone());
    let prepared = prepare(&mut session, &orderable, &documents).unwrap();
    documents.extend(serde_json::from_value::<sources::Documents>(prepared["documents"].clone()).unwrap());
    load_rating_source(&mut session, &documents);

    for (id, manufacturer, expected) in [("unrated", false, None), ("rated", false, Some(&rating)), ("unrated", true, Some(&rating))] {
        let resolved = session.execute(sources::RESOLVE_PART, &json!({"library":LIBRARY,"definition":id,"definition_revision":1})).unwrap();
        if id == "rated" {
            assert_eq!(resolved["part"]["fixed_parameters"][VOLTAGE_RATING_FIELD], number);
        }
        let mut params = json!({"library":LIBRARY,"definition":id,"definition_revision":1,"definition_digest":resolved["digest"],"x_nm":0,"y_nm":0});
        if manufacturer {
            let record = session.libraries().manufacturer_part(&Reference { library: LIBRARY.into(), id: "test-mpn".into(), revision: 1 }).unwrap();
            assert_eq!(record.specifications.get(VOLTAGE_RATING_FIELD), expected);
            params["manufacturer"] = json!(record.reference());
            params["manufacturer_digest"] = json!(sources::manufacturer_digest(&record));
        }
        apply(&mut session, PLACE_PART, params);
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        let reopened = decode_documents(&saved).unwrap();
        let selection = reopened.components().next().unwrap().part.as_ref().unwrap();
        assert_eq!(
            selection.manufacturer.as_ref().map_or_else(
                || selection.resolved.definition.fixed_parameters.get(VOLTAGE_RATING_FIELD),
                |record| record.specifications.get(VOLTAGE_RATING_FIELD),
            ),
            expected
        );
        assert_eq!(encode_documents(reopened).unwrap(), saved);
        if id == "unrated" && !manufacturer {
            let project = session.project().unwrap();
            let placement = &project.symbols()[0];
            let wrong_value =
                json!({"symbol":placement.id(),"reference":project.reference(placement),"value":"50 A","expected_revision":session.revision()});
            let before = session.snapshot().unwrap();
            assert_rating_error(session.apply(crate::SET_SYMBOL_FIELDS, &wrong_value).unwrap_err(), "Expected V or a compatible SI-prefixed unit");
            assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
            apply(
                &mut session,
                crate::SET_SYMBOL_FIELDS,
                json!({"symbol":wrong_value["symbol"],"reference":wrong_value["reference"],"value":"50 V"}),
            );
            let updated = session.project().unwrap().components().next().unwrap().part.as_ref().unwrap();
            assert_eq!(updated.primary_value().as_deref(), Some("50 V"));
            assert!(!updated.resolved.definition.fixed_parameters.contains_key(VOLTAGE_RATING_FIELD));
            session.apply("edit.undo", &Json::Null).unwrap();
            check_unrated_instance_requirement(&mut session, &number);
        }
        session.apply("edit.undo", &Json::Null).unwrap();
        assert!(session.project().unwrap().components().next().is_none());
    }
}

/// An instance requirement is not a part rating, even after saving and reopening.
fn check_unrated_instance_requirement(session: &mut Session, number: &Json) {
    let instance = session.project().unwrap().components().next().unwrap().id();
    let revision = session.revision();
    let before = session.snapshot().unwrap();
    let wrong_unit = json!({"component":instance,"expected_revision":revision,"requirements":{
        (VOLTAGE_RATING_FIELD):{"operator":"at_least","value":{"type":"quantity","value":"50","unit":"A"}}
    }});
    assert_voltage_unit_error(session.apply(SET_PART, &wrong_unit).unwrap_err());
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));

    apply(session, SET_PART, json!({"component":instance,"requirements":{(VOLTAGE_RATING_FIELD):{"operator":"at_least","value":number}}}));
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    let selection = reopened.components().next().unwrap().part.as_ref().unwrap();
    assert_eq!(json!(selection.requirements[VOLTAGE_RATING_FIELD]), json!({"operator":"at_least","value":number}));
    assert!(!selection.resolved.definition.fixed_parameters.contains_key(VOLTAGE_RATING_FIELD));
    session.apply("edit.undo", &Json::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), encode_documents(before).unwrap());
}

/// An existing field ID is not reserved; text and voltage definitions both reopen unchanged.
#[test]
fn category_defined_voltage_field_id_keeps_its_existing_meaning() {
    const LEGACY_ID: &str = "pcbcraft:voltage-rating";
    for (value_type, value) in [
        (FieldType::Text {}, Value::Text { value: "vendor label".into() }),
        (FieldType::Quantity { quantity: Quantity::Voltage, unit: "V".into() }, Value::Quantity { value: "50".into(), unit: "V".into() }),
    ] {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":false})).unwrap();
        let mut category = rating_category();
        category.fields = BTreeMap::from([(LEGACY_ID.into(), Field { name: "Legacy category value".into(), value_type })]);
        let mut documents = sources::Documents::from([
            ("library.json".into(), new_library(LIBRARY, "Synthetic legacy category", LICENSE).unwrap()),
            ("categories/capacitor.json".into(), category.encode().unwrap()),
        ]);
        load_rating_source(&mut session, &documents);
        let mut draft = Draft::new_part(Reference { library: "basic-components".into(), id: "capacitor".into(), revision: 3 }).unwrap();
        let Draft::Part(part) = &mut draft else { panic!("expected generic draft") };
        part.id = "legacy-cap".into();
        part.category = Some(CategoryBinding { definition: category.reference(), primary_value: None });
        part.fixed_parameters.insert(LEGACY_ID.into(), value.clone());
        let prepared = prepare(&mut session, &draft, &documents).unwrap();
        documents.extend(serde_json::from_value::<sources::Documents>(prepared["documents"].clone()).unwrap());
        load_rating_source(&mut session, &documents);
        let resolved = session.execute(sources::RESOLVE_PART, &json!({"library":LIBRARY,"definition":"legacy-cap","definition_revision":1})).unwrap();
        apply(
            &mut session,
            PLACE_PART,
            json!({"library":LIBRARY,"definition":"legacy-cap","definition_revision":1,"definition_digest":resolved["digest"],"x_nm":0,"y_nm":0}),
        );
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        let reopened = decode_documents(&saved).unwrap();
        let selection = reopened.components().next().unwrap().part.as_ref().unwrap();
        assert_eq!(selection.resolved.category.as_deref(), Some(&category));
        assert_eq!(selection.resolved.definition.fixed_parameters[LEGACY_ID], value);
        assert_eq!(encode_documents(reopened).unwrap(), saved);
    }
}

/// Canonical-but-incompatible quantities identify both the field and the unit mismatch.
fn assert_voltage_unit_error(error: crate::CommandError) {
    assert_rating_error(error, "Expected unit V, got A");
}

/// A refusal names the stable rating field and identifies the relevant unit problem.
fn assert_rating_error(error: crate::CommandError, detail: &str) {
    let error = error.to_string();
    assert!(error.contains(VOLTAGE_RATING_FIELD) && error.contains(detail), "{error}");
}
