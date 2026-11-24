use super::*;
use crate::library::sources::{Catalog, Documents, Kind, NativeSource};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

/// Shared definitions must preserve the published category vocabulary and native
/// command value shape; parsing a standard quantity remains a read-only operation.
#[test]
fn shared_registry_matches_native_schema_and_command_values() {
    use pcbcraft_units::quantity::{CanonicalQuantity, standard_quantities};
    use std::collections::BTreeSet;
    let schema: serde_json::Value = serde_json::from_str(include_str!("../../../../../docs/schemas/library-common-v1.schema.json")).unwrap();
    let published: BTreeSet<_> = schema["$defs"]["field_type"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|field| {
            Some((field["properties"]["quantity"]["const"].as_str()?.to_owned(), field["properties"]["unit"]["const"].as_str()?.to_owned()))
        })
        .collect();
    let shared: BTreeSet<_> = standard_quantities()
        .map(|definition| (serde_json::to_value(definition.quantity).unwrap().as_str().unwrap().to_owned(), definition.canonical_unit.to_owned()))
        .collect();
    assert_eq!(shared, published);
    assert_eq!(shared.len(), Quantity::ALL.len());
    let stored_units: BTreeSet<_> = schema["$defs"]["parameter_value"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| variant["properties"]["type"]["const"] == "quantity")
        .unwrap()["properties"]["unit"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|unit| unit.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(stored_units, standard_quantities().map(|definition| definition.canonical_unit.to_owned()).collect());
    let mut session = crate::Session::default();
    let revision = session.revision();
    for definition in standard_quantities() {
        let canonical = CanonicalQuantity::new("0.1", definition.canonical_unit).unwrap();
        let native = Value::from(canonical);
        native.validate().unwrap();
        let expected = json!({"type":"quantity","value":"0.1","unit":definition.canonical_unit});
        assert_eq!(serde_json::to_value(native).unwrap(), expected);
        let result = session
            .execute("library.parse_quantity", &json!({"quantity":definition.quantity,"value":format!("0.1 {}", definition.canonical_unit)}))
            .unwrap();
        assert_eq!(result, expected);
        assert_eq!(session.revision(), revision);
    }
}

/// Original native data, shared with schema and folder/bundle verification.
fn documents() -> Documents {
    [
        ("library.json", include_str!("../../../../../examples/native-parts/library.json")),
        ("symbols/resistor.json", include_str!("../../../../../examples/native-parts/symbols/resistor.json")),
        ("footprints/resistor-0603.json", include_str!("../../../../../examples/native-parts/footprints/resistor-0603.json")),
        ("categories/resistor.json", include_str!("../../../../../examples/native-parts/categories/resistor.json")),
        ("parts/resistor.json", include_str!("../../../../../examples/native-parts/parts/resistor.json")),
    ]
    .into_iter()
    .map(|(path, text)| (path.into(), text.into()))
    .collect()
}

/// Native commands load, search and resolve the same exact definitions without editing project data.
#[test]
fn actual_source_commands_resolve_parts_without_project_mutation() {
    let bundle = json!({"format":"pcbcraft-library-bundle/1","documents":documents()}).to_string();
    let uploaded = NativeSource::from_bundle(bundle.as_bytes()).unwrap();
    assert_eq!(uploaded.items().len(), 4);
    assert!(uploaded.problems().is_empty());
    let mut session = crate::Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let before = crate::persistence::encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    let loaded = session.execute("library.load_source", &json!({"source":"parts-demo","documents":documents()})).unwrap();
    let source = loaded.as_array().unwrap().iter().find(|s| s["library"] == "parts-demo").unwrap();
    assert_eq!(source["parts"], 1);
    assert_eq!(source["categories"], 1);
    assert_eq!(source["problems"], json!([]));
    let found = session.execute("library.search", &json!({"kind":"part","query":"resistor"})).unwrap();
    assert_eq!(found[0]["resolution"]["status"], "resolved");
    let resolved = session.execute("library.resolve_part", &json!({"library":"parts-demo","definition":"resistor","definition_revision":1})).unwrap();
    assert_eq!(resolved["symbol"]["format"], "pcbcraft-symbol/3");
    assert_eq!(resolved["category"]["fields"]["pcbcraft:resistance"]["value_type"]["unit"], "Ohm");
    assert_eq!(resolved["footprints"]["0603"]["id"], "resistor-0603");
    assert_eq!(session.revision(), revision);
    assert_eq!(crate::persistence::encode_documents(session.snapshot().unwrap()).unwrap(), before);
    assert!(session.execute("library.resolve_part", &json!({"library":"parts-demo","definition":"resistor","definition_revision":2})).is_err());
}

/// A missing dependency remains a useful unresolved listing, never a guessed latest revision.
#[test]
fn missing_and_conflicting_dependencies_prevent_resolution_and_recover_on_removal() {
    let mut docs = documents();
    let category = docs.remove("categories/resistor.json").unwrap();
    let mut catalog = Catalog::default();
    catalog.install("demo", Arc::new(NativeSource::load(&docs).unwrap())).unwrap();
    let reference = Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
    assert_eq!(catalog.search("", Some(Kind::Part)).len(), 1);
    assert!(catalog.resolve_part(&reference).unwrap_err().contains("Category"));
    docs.insert("categories/resistor.json".into(), category);
    catalog.install("demo", Arc::new(NativeSource::load(&docs).unwrap())).unwrap();
    assert!(catalog.resolve_part(&reference).is_ok());
    let mut conflicting = docs.clone();
    let mut footprint: serde_json::Value = serde_json::from_str(&conflicting["footprints/resistor-0603.json"]).unwrap();
    footprint["name"] = json!("Conflicting copy");
    conflicting.insert("footprints/resistor-0603.json".into(), footprint.to_string());
    catalog.install("conflicting", Arc::new(NativeSource::load(&conflicting).unwrap())).unwrap();
    assert!(catalog.resolve_part(&reference).unwrap_err().contains("Conflicting Footprint"));
    catalog.remove("conflicting").unwrap();
    assert!(catalog.resolve_part(&reference).is_ok());
    catalog.mark_unavailable("demo");
    assert!(catalog.resolve_part(&reference).is_err());
}

/// Multiple lands per terminal and explicit non-electrical pads retain identity, including leading zeros.
#[test]
fn package_maps_cover_every_terminal_and_pad_once() {
    let docs = documents();
    let symbol = crate::library::parse_symbol(&docs["symbols/resistor.json"]).unwrap();
    let mut footprint = (*crate::board::definition::read_saved(docs["footprints/resistor-0603.json"].as_bytes()).unwrap()).clone();
    let mut pad = footprint.pads[0].clone();
    pad.number = "EP".into();
    footprint.pads.push(pad.clone());
    pad.number = "01".into();
    footprint.pads.push(pad);
    let mut package = PartDefinition::read(&docs["parts/resistor.json"]).unwrap().packages["0603"].clone();
    package.mapping.get_mut("1").unwrap().push("EP".into());
    package.unconnected_pads.push("01".into());
    package.validate_definitions(&symbol, &footprint).unwrap();
    package.unconnected_pads.push("EP".into());
    let both = Err("Pad EP cannot be both mapped and unconnected, or repeated.".to_owned());
    assert_eq!(package.validate_definitions(&symbol, &footprint), both);
    let assigned: BTreeMap<String, mapping::PadIds> =
        package.mapping.iter().map(|(terminal, pads)| (terminal.clone(), mapping::PadIds::from(pads.clone()))).collect();
    assert_eq!(mapping::validate(&assigned, &package.unconnected_pads, &symbol, &footprint), both);
    package.unconnected_pads.pop();
    package.mapping.get_mut("2").unwrap().push("EP".into());
    assert!(package.validate_definitions(&symbol, &footprint).is_err());
    package.mapping.get_mut("2").unwrap().pop();
    package.unconnected_pads.clear();
    assert!(package.validate_definitions(&symbol, &footprint).is_err());
    package.unconnected_pads.push("01".into());
    package.mapping.remove("2");
    assert!(package.validate_definitions(&symbol, &footprint).is_err());
}

/// A package's independent unconnected-pad vector accepts 1,024 unique pads and names the next count.
#[test]
fn package_unconnected_pad_vector_stops_at_the_footprint_limit() {
    let mut package = PartDefinition::read(&documents()["parts/resistor.json"]).unwrap().packages["0603"].clone();
    package.mapping.clear();
    package.unconnected_pads = (1..=1024).map(|n| format!("P{n}")).collect();
    assert_eq!(package.validate(), Ok(()));
    package.unconnected_pads.push("P1025".into());
    assert_eq!(package.validate(), Err("A package's `unconnected_pads` holds at most 1024 pads, not 1025.".into()));
}

/// One native package terminal can own the entire footprint, independently of command-array limits.
#[test]
fn package_terminal_pad_vector_stops_at_the_footprint_limit() {
    let mut package = PartDefinition::read(&documents()["parts/resistor.json"]).unwrap().packages["0603"].clone();
    package.mapping = BTreeMap::from([("1".into(), (1..=1024).map(|n| format!("P{n}")).collect())]);
    package.unconnected_pads.clear();
    assert_eq!(package.validate(), Ok(()));
    package.mapping.get_mut("1").unwrap().push("P1025".into());
    assert_eq!(package.validate(), Err("A package's `mapping` for terminal 1 requires 1–1024 pads, not 1025.".into()));
}

/// The native package pad budget includes both mapped and explicitly unconnected identities.
#[test]
fn package_mapped_and_unconnected_pads_share_the_footprint_limit() {
    let mut package = PartDefinition::read(&documents()["parts/resistor.json"]).unwrap().packages["0603"].clone();
    package.mapping = BTreeMap::from([("1".into(), (1..=1023).map(|n| format!("P{n}")).collect())]);
    package.unconnected_pads = vec!["P1024".into()];
    assert_eq!(package.validate(), Ok(()));
    package.unconnected_pads.push("P1025".into());
    assert_eq!(package.validate(), Err("A package has at most 1024 pads, not 1025.".into()));
}

/// Fixed part parameters require declared, correctly typed category fields and canonical quantities.
#[test]
fn typed_fixed_values_and_uncategorized_parts_have_distinct_meanings() {
    let docs = documents();
    let mut category = Category::read(&docs["categories/resistor.json"]).unwrap();
    category.fields.insert("parts-demo:description".into(), Field { name: "Description".into(), value_type: FieldType::Text {} });
    category.fields.insert("parts-demo:shielded".into(), Field { name: "Shielded".into(), value_type: FieldType::Boolean {} });
    let mut part = PartDefinition::read(&docs["parts/resistor.json"]).unwrap();
    part.fixed_parameters.insert("parts-demo:description".into(), Value::Text { value: "Original generic part".into() });
    part.fixed_parameters.insert("parts-demo:shielded".into(), Value::Boolean { value: false });
    part.fixed_parameters.insert("pcbcraft:resistance".into(), Value::Quantity { value: "0.1".into(), unit: "Ohm".into() });
    part.validate_category(Some(&category)).unwrap();
    for value in [
        Value::Text { value: "MPN-1".into() },
        Value::Quantity { value: "0.1".into(), unit: "V".into() },
        Value::Quantity { value: "0.10".into(), unit: "Ohm".into() },
    ] {
        part.fixed_parameters.insert("pcbcraft:resistance".into(), value);
        assert!(part.validate_category(Some(&category)).is_err());
    }
    part.fixed_parameters.clear();
    part.category = None;
    part.packages.clear();
    part.validate_category(None).unwrap();
    let encoded = part.encode().unwrap();
    assert_eq!(PartDefinition::read(&encoded).unwrap().encode().unwrap(), encoded);
    let encoded = category.encode().unwrap();
    assert_eq!(Category::read(&encoded).unwrap().encode().unwrap(), encoded);
}

/// Closed formats reject metadata from other record kinds, duplicate IDs and incompatible versions.
#[test]
fn strict_native_files_reject_ambiguous_or_misowned_content() {
    let docs = documents();
    let mut part: serde_json::Value = serde_json::from_str(&docs["parts/resistor.json"]).unwrap();
    for (key, value) in [("manufacturer", json!("Vendor")), ("mpn", json!("ABC")), ("pads", json!([])), ("format", json!("pcbcraft-part/99"))] {
        let mut changed = part.clone();
        changed[key] = value;
        assert!(PartDefinition::read(&changed.to_string()).is_err());
    }
    let package = part["packages"]["0603"].to_string();
    part["packages"] = json!({});
    let mut encoded = part.to_string();
    encoded = encoded.replace("\"packages\":{}", &format!("\"packages\":{{\"same\":{package},\"same\":{package}}}"));
    assert!(PartDefinition::read(&encoded).unwrap_err().contains("duplicate native record identities"));
    let mut category: serde_json::Value = serde_json::from_str(&docs["categories/resistor.json"]).unwrap();
    category["fields"]["pcbcraft:resistance"]["value_type"]["unit"] = json!("V");
    assert!(Category::read(&category.to_string()).is_err());
    category["fields"]["pcbcraft:resistance"]["value_type"] = json!({"type":"text","unit":"V"});
    assert!(Category::read(&category.to_string()).is_err());
}

/// Cached resolution is stable within a generation and cannot survive a conflicting field meaning.
#[test]
fn category_field_conflicts_invalidate_resolution_without_changing_old_snapshots() {
    let docs = documents();
    let mut catalog = Catalog::default();
    catalog.install("demo", Arc::new(NativeSource::load(&docs).unwrap())).unwrap();
    let reference = Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
    assert!(std::ptr::eq(catalog.resolve_part(&reference).unwrap(), catalog.resolve_part(&reference).unwrap()));
    let old = catalog.clone();
    let mut category: serde_json::Value = serde_json::from_str(&docs["categories/resistor.json"]).unwrap();
    category["id"] = json!("incompatible-field");
    category["fields"]["pcbcraft:resistance"]["value_type"] = json!({"type":"quantity","quantity":"voltage","unit":"V"});
    let changed = Documents::from([("library.json".into(), docs["library.json"].clone()), ("categories/other.json".into(), category.to_string())]);
    catalog.install("conflicting-field", Arc::new(NativeSource::load(&changed).unwrap())).unwrap();
    assert!(catalog.resolve_part(&reference).unwrap_err().contains("Conflicting type definitions"));
    assert!(old.resolve_part(&reference).is_ok());
    catalog.remove("conflicting-field").unwrap();
    assert!(catalog.resolve_part(&reference).is_ok());
}

/// Symbol defaults survive native authoring export, project pinning and offline save/reopen.
#[test]
fn symbol_category_binding_round_trips_without_reinterpreting_older_formats() {
    let docs = documents();
    let mut symbol = crate::library::parse_symbol(&docs["symbols/resistor.json"]).unwrap();
    symbol.library = "parts-demo".into();
    symbol.provenance = Some(crate::library::sources::Provenance { library_name: "Native Parts Demo".into(), license: "MIT OR Apache-2.0".into() });
    let item = crate::authoring::Item::from_symbol(&symbol).unwrap();
    let exported = item.export_native().unwrap();
    assert_eq!(exported["format"], "pcbcraft-symbol/3");
    assert_eq!(exported["category"], json!(symbol.category));
    let saved = serde_json::to_value(crate::library::saved_copy(&symbol)).unwrap();
    assert_eq!(*crate::library::read_saved_copy(saved).unwrap(), symbol);
    let mut old: serde_json::Value = serde_json::from_str(&docs["symbols/resistor.json"]).unwrap();
    old["format"] = json!("pcbcraft-symbol/2");
    assert!(crate::library::parse_symbol(&old.to_string()).is_err());
    old.as_object_mut().unwrap().remove("category");
    assert!(crate::library::parse_symbol(&old.to_string()).is_ok());
}

/// Plain canonical decimals preserve precision and reject alternate spellings, floats and exponents.
#[test]
fn canonical_decimals_are_exact_bounded_and_language_independent() {
    for value in ["0", "1", "-1", "0.1", "-0.1", "100000000000000000000000000000000000", "0.000000000000000001"] {
        assert!(decimal(value), "{value}");
    }
    for value in [
        "",
        "-0",
        "00",
        "01",
        "1.",
        ".1",
        "1.0",
        "1e3",
        "NaN",
        "1 V",
        "+1",
        " 1",
        "1\n",
        "0.0000000000000000001",
        "1000000000000000000000000000000000000",
    ] {
        assert!(!decimal(value), "{value}");
    }
}

/// Equivalent input spellings yield byte-equivalent canonical values through the shared command.
#[test]
fn quantity_input_normalizes_prefixes_units_and_precision_without_floats() {
    let mut session = crate::Session::default();
    let expected = json!({"type":"quantity","value":"0.1","unit":"Ohm"});
    for input in ["100m", "0.1R", "100 mΩ", "100 milli ohms", "0.100 Ohm", "1e-1Ω"] {
        let parsed = session.execute("library.parse_quantity", &json!({"quantity":"resistance","value":input})).unwrap();
        assert_eq!(parsed, expected, "{input}");
    }
    for (quantity, input, value, unit) in [
        (Quantity::Resistance, "1M", "1000000", "Ohm"),
        (Quantity::Capacitance, "100nF", "0.0000001", "F"),
        (Quantity::Length, "100 mm", "0.1", "m"),
        (Quantity::Ratio, "1%", "0.01", "1"),
        (Quantity::Voltage, "-1.2 mV", "-0.0012", "V"),
    ] {
        assert_eq!(
            serde_json::to_value(quantity.parse(input).map(Value::from).unwrap()).unwrap(),
            json!({"type":"quantity","value":value,"unit":unit})
        );
    }
    for input in ["1 V", "MPN-1", "1e999", "1.2.3", "1e-19", "", "NaN", "100 μV"] {
        assert!(Quantity::Resistance.parse(input).is_err(), "{input}");
    }
    assert_eq!(Quantity::Resistance.parse("-0.000 Ω").map(Value::from).unwrap(), Value::Quantity { value: "0".into(), unit: "Ohm".into() });
}

/// A part's single exact symbol reference includes every unit and its complete physical map.
#[test]
fn multiunit_part_resolves_one_symbol_and_whole_package_map() {
    let mut documents = documents();
    documents.insert("symbols/dual-amplifier.json".into(), include_str!("../../../../../examples/native-parts/symbols/dual-amplifier.json").into());
    documents.insert("parts/dual-amplifier.json".into(), include_str!("../../../../../examples/native-parts/parts/dual-amplifier.json").into());
    documents.insert(
        "footprints/illustrative-six-pad.json".into(),
        include_str!("../../../../../examples/native-parts/footprints/illustrative-six-pad.json").into(),
    );
    let mut catalog = Catalog::default();
    let source = Arc::new(NativeSource::load(&documents).unwrap());
    assert!(source.problems().is_empty(), "{:?}", source.problems());
    catalog.install("multiunit", source).unwrap();
    let reference = Reference { library: "parts-demo".into(), id: "dual-amplifier".into(), revision: 1 };
    let resolved = catalog.resolve_part(&reference).unwrap();
    assert_eq!(resolved.symbol.as_ref().unwrap().units().len(), 2);
    assert_eq!(resolved.symbol.as_ref().unwrap().pins().len(), 6);
    assert_eq!(resolved.definition.packages["six-pad"].mapping.len(), 6);
    assert_eq!(resolved.footprints.len(), 1);
    assert_eq!(resolved.inspect()["symbol"]["format"], "pcbcraft-symbol/4");
}

/// Canonical decimal comparisons retain all 18 fractional places across signs and magnitude changes.
#[test]
fn requirements_preserve_precision_types_and_unknown_specifications() {
    let category = Category::read(include_str!("../../../../../examples/native-parts/categories/resistor.json")).unwrap();
    let field = &category.fields["pcbcraft:resistance"];
    let quantity = |value: &str| Value::Quantity { value: value.into(), unit: "Ohm".into() };
    let range = Requirement::Range { minimum: quantity("-10.000000000000000001"), maximum: quantity("0.000000000000000001") };
    range.validate(field).unwrap();
    for value in ["-10", "-0.000000000000000001", "0", "0.000000000000000001"] {
        assert!(range.matches(&quantity(value)), "{value}");
    }
    for value in ["-10.000000000000000002", "0.000000000000000002", "100"] {
        assert!(!range.matches(&quantity(value)), "{value}");
    }
    assert!(!range.matches(&Value::Quantity { value: "0".into(), unit: "V".into() }));
    assert!(Requirement::Range { minimum: quantity("10"), maximum: quantity("9") }.validate(field).is_err());
    let one = Requirement::OneOf { values: vec![quantity("0.1"), quantity("100")] };
    one.validate(field).unwrap();
    assert!(one.matches(&quantity("0.1")));
    assert!(!one.matches(&quantity("1")));
    assert!(Requirement::OneOf { values: vec![quantity("0.1"), quantity("0.1")] }.validate(field).is_err());
    assert!(Requirement::AtMost { value: Value::Choice { value: "thin-film".into() } }.validate(&category.fields["parts-demo:technology"]).is_err());
    assert_eq!(field.parse_input("100 mΩ").unwrap(), quantity("0.1"));
}

/// Manufacturer records accept only canonical declared fields and explicit native package relationships.
#[test]
fn manufacturer_records_validate_metadata_and_category_meaning() {
    let text = include_str!("../../../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json");
    let record = ManufacturerPart::read(text).unwrap();
    assert_eq!(ManufacturerPart::read(&record.encode().unwrap()).unwrap(), record);
    let mut catalog = Catalog::default();
    catalog.install("test", Arc::new(NativeSource::load(&documents()).unwrap())).unwrap();
    let part = catalog.resolve_part(&record.part).unwrap();
    record.validate_part(part).unwrap();
    let mut bad = record.clone();
    bad.specifications.insert("pcbcraft:resistance".into(), Value::Quantity { value: "100".into(), unit: "mOhm".into() });
    assert!(bad.validate().is_err());
    bad = record.clone();
    bad.package = "missing".into();
    assert!(bad.validate_part(part).is_err());
    bad = record.clone();
    bad.specifications.insert("vendor:unknown".into(), Value::Boolean { value: true });
    assert!(bad.validate_part(part).is_err());
    for url in ["https://", "file:///tmp/example.pdf", "https://example.invalid/a b.pdf"] {
        bad = record.clone();
        bad.datasheet = Some(url.into());
        assert!(bad.validate().is_err(), "{url}");
    }
    assert!(
        ManufacturerPart::read(
            &text.replace(
                "\"specifications\": {",
                "\"specifications\": {\"pcbcraft:tolerance\":{\"type\":\"quantity\",\"value\":\"1\",\"unit\":\"1\"},"
            )
        )
        .is_err()
    );
}

/// Native textual values keep their literal spelling when quantity formatting is shared.
#[test]
fn native_text_display_remains_literal() {
    assert_eq!(Value::Text { value: "DNP".into() }.display(), "DNP");
}

/// The original /1 document, and a /2 copy of it, are the unchanged encoding baselines.
fn legacy_bytes() -> &'static str {
    include_str!("../../../../../examples/native-parts/parts/resistor.json")
}
fn plain_part() -> PartDefinition {
    let mut part = PartDefinition::read(legacy_bytes()).unwrap();
    part.format = PART_FORMAT.into();
    part
}
fn describe(part: &mut PartDefinition) {
    part.description = Some("Precision thin-film resistor".into());
    part.keywords = vec!["precision".into(), "thin film".into()];
    part.links = vec![PartLink { label: "Datasheet".into(), url: "https://example.invalid/ds.pdf".into() }];
}
fn metadata_json() -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(legacy_bytes()).unwrap();
    value["format"] = json!(METADATA_PART_FORMAT);
    value["description"] = json!("Precision thin-film resistor");
    value["keywords"] = json!(["precision", "thin film"]);
    value["links"] = json!([{"label":"Datasheet","url":"https://example.invalid/ds.pdf"}]);
    value
}
fn read_json(value: &serde_json::Value) -> Result<PartDefinition, String> {
    PartDefinition::read(&value.to_string())
}

/// Retained /1, plain /2 and rated /4 bytes stay byte-for-byte stable, while /3 remains unused.
#[test]
fn older_part_formats_stay_byte_for_byte_stable() {
    let legacy = legacy_bytes();
    let part = PartDefinition::read(legacy).unwrap();
    assert_eq!(part.format, LEGACY_PART_FORMAT);
    assert_eq!(part.encode().unwrap(), legacy);

    let plain = plain_part();
    let plain_text = plain.encode().unwrap();
    assert_eq!(PartDefinition::read(&plain_text).unwrap().encode().unwrap(), plain_text);
    let plain_value: serde_json::Value = serde_json::from_str(&plain_text).unwrap();
    assert!(plain_value.get("description").is_none() && plain_value.get("keywords").is_none() && plain_value.get("links").is_none());

    let mut rated = plain.clone();
    rated.set_supply_range("1", "4.5 V", "36 V").unwrap();
    assert_eq!(rated.format, RATED_PART_FORMAT);
    let rated_text = rated.encode().unwrap();
    assert_eq!(PartDefinition::read(&rated_text).unwrap().encode().unwrap(), rated_text);
    let rated_value: serde_json::Value = serde_json::from_str(&rated_text).unwrap();
    assert_eq!(rated_value["terminal_ratings"]["1"]["supply_min"]["value"], "4.5");
    assert!(rated_value.get("description").is_none());

    // Removing the last rating returns an authored /4 draft to exactly the /2 record.
    rated.set_supply_range("1", "", "").unwrap();
    assert_eq!(rated.format, PART_FORMAT);
    assert_eq!(rated.encode().unwrap(), plain_text);

    let mut sentinel = plain;
    sentinel.format = "pcbcraft-part/3".into();
    assert!(sentinel.encode().unwrap_err().contains("Unsupported native format"));
    let mut forged = metadata_json();
    forged["format"] = json!("pcbcraft-part/3");
    assert!(read_json(&forged).is_err());
}

/// Every upgrade and downgrade edge of the format state machine selects the minimal envelope.
#[test]
fn part_format_state_machine_covers_all_transitions() {
    for (initial, rating, metadata, expected) in [
        (LEGACY_PART_FORMAT, false, false, LEGACY_PART_FORMAT),
        (LEGACY_PART_FORMAT, true, false, RATED_PART_FORMAT),
        (LEGACY_PART_FORMAT, false, true, METADATA_PART_FORMAT),
        (LEGACY_PART_FORMAT, true, true, METADATA_PART_FORMAT),
        (PART_FORMAT, false, false, PART_FORMAT),
        (PART_FORMAT, true, false, RATED_PART_FORMAT),
        (PART_FORMAT, false, true, METADATA_PART_FORMAT),
        (RATED_PART_FORMAT, false, false, PART_FORMAT),
        (RATED_PART_FORMAT, true, false, RATED_PART_FORMAT),
        (RATED_PART_FORMAT, true, true, METADATA_PART_FORMAT),
        (METADATA_PART_FORMAT, false, true, METADATA_PART_FORMAT),
        (METADATA_PART_FORMAT, true, true, METADATA_PART_FORMAT),
        (METADATA_PART_FORMAT, false, false, PART_FORMAT),
        (METADATA_PART_FORMAT, true, false, RATED_PART_FORMAT),
        ("pcbcraft-part/3", false, true, METADATA_PART_FORMAT),
    ] {
        let mut part = plain_part();
        if rating {
            part.set_supply_range("1", "4.5 V", "36 V").unwrap();
        }
        if metadata {
            describe(&mut part);
        }
        part.format = initial.into();
        part.select_format();
        assert_eq!(part.format, expected, "from {initial} with rating={rating} metadata={metadata}");
        part.validate().unwrap_or_else(|error| panic!("{initial} -> {expected}: {error}"));
    }
    // A legacy record without its symbol is an authored /2, not a broken /1.
    let mut physical = PartDefinition::read(legacy_bytes()).unwrap();
    physical.symbol = None;
    for package in physical.packages.values_mut() {
        package.mapping.clear();
    }
    physical.select_format();
    assert_eq!(physical.format, PART_FORMAT);
    physical.validate().unwrap();
}

/// Description, keyword and link fields accept exact bounds and reject malformed or ambiguous input.
#[test]
fn part_metadata_fields_are_bounded_canonical_and_null_intolerant() {
    let valid = metadata_json();
    let part = read_json(&valid).unwrap();
    assert_eq!(part.format, METADATA_PART_FORMAT);
    let encoded = part.encode().unwrap();
    assert_eq!(PartDefinition::read(&encoded).unwrap(), part);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&encoded).unwrap()["keywords"][0], "precision");

    for (field, value) in [
        ("description", json!(null)),
        ("description", json!("")),
        ("description", json!(" leading")),
        ("description", json!("line\nbreak")),
        ("description", json!("x".repeat(MAX_DESCRIPTION_CHARS + 1))),
        ("keywords", json!(null)),
        ("keywords", json!([])),
        ("keywords", json!([""])),
        ("keywords", json!(["   "])),
        ("keywords", json!(["x".repeat(MAX_KEYWORD_CHARS + 1)])),
        ("keywords", json!(["tab\there"])),
        ("keywords", json!(vec!["k"; MAX_KEYWORDS + 1])),
        ("keywords", json!(["Precision", "precision"])),
        ("keywords", json!(["Precision", " precision "])),
        ("links", json!(null)),
        ("links", json!([])),
        ("links", json!([{"label":"","url":"https://example.invalid"}])),
        ("links", json!([{"label":"x".repeat(MAX_LINK_LABEL_CHARS + 1),"url":"https://example.invalid"}])),
        ("links", json!([{"label":"Datasheet","url":"https://example.invalid"},{"label":" datasheet ","url":"https://other.invalid"}])),
        ("links", json!((0..MAX_LINKS + 1).map(|_| json!({"label":"A","url":"https://example.invalid"})).collect::<Vec<_>>())),
        ("links", json!([{"label":"Datasheet","url":"file:///tmp/x.pdf"}])),
    ] {
        let mut changed = valid.clone();
        changed[field] = value;
        assert!(read_json(&changed).is_err(), "{field}: {changed}");
    }

    // Exact limits still round-trip, and keyword/label whitespace is normalized on storage.
    let mut edge = valid.clone();
    edge["description"] = json!("x".repeat(MAX_DESCRIPTION_CHARS));
    edge["keywords"] = json!((0..MAX_KEYWORDS).map(|i| format!("k{i}")).collect::<Vec<_>>());
    edge["links"] = json!((0..MAX_LINKS).map(|i| json!({"label":format!("L{i}"),"url":format!("https://example.invalid/{i}")})).collect::<Vec<_>>());
    read_json(&edge).unwrap();
    let mut trimmed = valid.clone();
    trimmed["keywords"] = json!(["  Precision  "]);
    trimmed["links"] = json!([{"label":"  Datasheet  ","url":"http://example.invalid/a?b=1#c"}]);
    let stored = read_json(&trimmed).unwrap();
    assert_eq!(stored.keywords, ["Precision"]);
    assert_eq!(stored.links[0].label, "Datasheet");
    assert_eq!(stored.encode().unwrap(), PartDefinition::read(&stored.encode().unwrap()).unwrap().encode().unwrap());
}

/// Link URLs use one shared HTTP(S) rule with the manufacturer datasheet field.
#[test]
fn part_link_urls_reject_every_malformed_form_but_accept_bounded_https() {
    let valid = metadata_json();
    let malformed: Vec<serde_json::Value> = vec![
        json!("ftp://example.invalid/x"),
        json!("file:///tmp/x.pdf"),
        json!("//example.invalid/x"),
        json!("http://"),
        json!("https://?x"),
        json!("https://#x"),
        json!("https://example.invalid:abc/datasheet"),
        json!("https://example.invalid:65536/datasheet"),
        json!("https://[bogus]/datasheet"),
        json!("https://example.invalid/a b"),
        json!("https://exa mple.invalid"),
        json!("https://example.invalid/\t"),
        json!("HTTP://example.invalid"),
        json!("example.invalid/x"),
        json!(format!("https://e.invalid/{}", "a".repeat(MAX_LINK_URL_CHARS - 18 + 1))),
    ];
    for url in malformed {
        let mut changed = valid.clone();
        changed["links"] = json!([{"label":"Datasheet","url":url}]);
        assert!(read_json(&changed).is_err(), "{url}");
    }
    let mut edge = valid.clone();
    edge["links"] = json!([{"label":"Datasheet","url":format!("https://e.invalid/{}", "a".repeat(MAX_LINK_URL_CHARS - 18))}]);
    let part = read_json(&edge).unwrap();
    assert_eq!(part.links[0].url.chars().count(), MAX_LINK_URL_CHARS);
    for url in ["http://example.invalid", "https://example.invalid/a?b=1#c", "https://example.invalid:8443/datasheet", "https://[::1]:443/x"] {
        let mut changed = valid.clone();
        changed["links"] = json!([{"label":"Datasheet","url":url}]);
        assert!(read_json(&changed).is_ok(), "{url}");
    }
    // The extracted helper keeps the manufacturer error and refusal behavior intact.
    let mut manufacturer: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json")).unwrap();
    for url in ["ftp://example.invalid/x", "https://", "https://example.invalid/a b"] {
        manufacturer["datasheet"] = json!(url);
        assert!(ManufacturerPart::read(&manufacturer.to_string()).is_err(), "{url}");
    }
}

/// Format ownership is strict: metadata requires /5, ratings require /4 or /5, and /5 needs content.
#[test]
fn part_metadata_and_ratings_require_their_exact_envelope() {
    for format in [LEGACY_PART_FORMAT, PART_FORMAT, RATED_PART_FORMAT] {
        let mut changed = metadata_json();
        changed["format"] = json!(format);
        assert!(read_json(&changed).unwrap_err().contains("pcbcraft-part/5"), "{format}");
    }
    let mut empty = metadata_json();
    for field in ["description", "keywords", "links"] {
        empty.as_object_mut().unwrap().remove(field);
    }
    let error = read_json(&empty).unwrap_err();
    assert!(error.contains("pcbcraft-part/5"), "{error}");

    let mut rated_plain: serde_json::Value = serde_json::from_str(&plain_part().encode().unwrap()).unwrap();
    rated_plain["terminal_ratings"] = json!({"1":{"supply_min":{"type":"quantity","value":"4.5","unit":"V"}}});
    assert!(read_json(&rated_plain).unwrap_err().contains("pcbcraft-part/4"));

    let mut unrated = metadata_json();
    unrated["terminal_ratings"] = json!({});
    assert!(read_json(&unrated).is_err());
    let mut rated = metadata_json();
    rated["terminal_ratings"] = json!({"1":{"supply_min":null}});
    assert!(read_json(&rated).is_err());
    let mut rated_and_described = metadata_json();
    rated_and_described["terminal_ratings"] = json!({"1":{"supply_min":{"type":"quantity","value":"4.5","unit":"V"}}});
    let part = read_json(&rated_and_described).unwrap();
    assert_eq!(part.format, METADATA_PART_FORMAT);
    assert_eq!(part.encode().unwrap(), PartDefinition::read(&part.encode().unwrap()).unwrap().encode().unwrap());

    // A rating always names a power_in terminal of the exact symbol, with or without metadata.
    let symbol = crate::library::parse_symbol(&documents()["symbols/resistor.json"]).unwrap();
    assert!(part.validate_rating_terminals(Some(&symbol)).unwrap_err().contains("power_in"));
    assert!(part.validate_rating_terminals(None).unwrap_err().contains("exact symbol"));
}

/// Search text is lowercase description plus normalized keywords, never link text.
#[test]
fn part_search_text_uses_only_description_and_keywords() {
    let mut part = plain_part();
    part.description = Some("Präzisions-Widerstand".into());
    part.keywords = vec!["LOW-Noise".into(), "Präzision".into()];
    part.links = vec![PartLink { label: "Supplier Portal".into(), url: "https://vendor.invalid/portal".into() }];
    let text = part.search_text();
    assert_eq!(text, "präzisions-widerstand low-noise präzision");
    assert!(!text.contains("supplier") && !text.contains("vendor"));
    let without = plain_part();
    assert_eq!(without.search_text(), "");
}

/// Catalog search finds a part by description or keyword (including Unicode case), not by link text.
#[test]
fn part_description_and_keywords_are_searchable_but_links_are_not() {
    let mut docs = documents();
    let mut part: serde_json::Value = serde_json::from_str(&docs["parts/resistor.json"]).unwrap();
    part["format"] = json!(METADATA_PART_FORMAT);
    part["description"] = json!("Cryogenic cryostat interface");
    part["keywords"] = json!(["Präzision", "low-noise"]);
    part["links"] = json!([{"label":"Supplier Portal","url":"https://vendor.invalid/portal"}]);
    docs.insert("parts/resistor.json".into(), part.to_string());
    let source = NativeSource::load(&docs).unwrap();
    assert!(source.problems().is_empty(), "{:?}", source.problems());
    let mut catalog = Catalog::default();
    catalog.install("search", Arc::new(source)).unwrap();
    for query in ["cryogenic", "CRYOSTAT", "präzision", "PRÄZISION", "LOW-NOISE", "interface"] {
        assert_eq!(catalog.search(query, Some(Kind::Part)).len(), 1, "{query}");
    }
    for query in ["supplier", "portal", "vendor.invalid", "datasheet"] {
        assert!(catalog.search(query, Some(Kind::Part)).is_empty(), "{query}");
    }
    assert_eq!(catalog.search("low-noise", Some(Kind::Symbol)).len(), 0);
}
