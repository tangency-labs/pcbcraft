use super::*;
use crate::library::sources::{Catalog, Documents, Kind, NativeSource};
use serde_json::json;
use std::sync::Arc;

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
    package.mapping.get_mut("2").unwrap().push("EP".into());
    assert!(package.validate_definitions(&symbol, &footprint).is_err());
    package.mapping.get_mut("2").unwrap().pop();
    package.unconnected_pads.clear();
    assert!(package.validate_definitions(&symbol, &footprint).is_err());
    package.unconnected_pads.push("01".into());
    package.mapping.remove("2");
    assert!(package.validate_definitions(&symbol, &footprint).is_err());
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
        assert_eq!(serde_json::to_value(quantity.parse(input).unwrap()).unwrap(), json!({"type":"quantity","value":value,"unit":unit}));
    }
    for input in ["1 V", "MPN-1", "1e999", "1.2.3", "1e-19", "", "NaN", "100 μV"] {
        assert!(Quantity::Resistance.parse(input).is_err(), "{input}");
    }
    assert_eq!(Quantity::Resistance.parse("-0.000 Ω").unwrap(), Value::Quantity { value: "0".into(), unit: "Ohm".into() });
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
    assert_eq!(resolved.symbol.units().len(), 2);
    assert_eq!(resolved.symbol.pins().len(), 6);
    assert_eq!(resolved.definition.packages["six-pad"].mapping.len(), 6);
    assert_eq!(resolved.footprints.len(), 1);
    assert_eq!(resolved.inspect()["symbol"]["format"], "pcbcraft-symbol/4");
}
