//! Upgrade retired supply annotations using ordinary native power-symbol placement.

use super::*;
use serde_json::{Value, json};

/// Convert retired power annotations in memory, preserving unaffected document bytes.
/// Validate the complete input with equivalent global anchors before running placement;
/// preserve authored wires and redirect only endpoints attached to a replaced marker.
/// Repeated reads allocate the same identities and never touch the filesystem.
pub fn migrate_power_markers(mut documents: Documents) -> Result<Documents, String> {
    documents::check_documents(&documents)?;
    let mut sanitized = documents.clone();
    let mut markers = Vec::new();
    for (path, bytes) in &documents {
        if !path.starts_with("sheets/") {
            continue;
        }
        let mut sheet = serde_json::from_slice::<UniqueValue>(bytes).map_err(|e| format!("{path}: {e}"))?.0;
        let mut changed = false;
        if let Some(labels) = sheet["net_labels"].as_array_mut() {
            for label in labels {
                let definition = match label["style"].as_str() {
                    Some("power") => "vcc",
                    Some("power_bar") => "vcc-bar",
                    Some("ground") => "gnd",
                    _ => continue,
                };
                markers.push((path.clone(), label.clone(), definition));
                label["style"] = json!("global");
                changed = true;
            }
        }
        if changed {
            sanitized.insert(path.clone(), serde_json::to_vec(&sheet).map_err(|e| e.to_string())?);
        }
    }
    if markers.is_empty() {
        return Ok(documents);
    }
    let original = decode_documents(&sanitized).map_err(|e| format!("Before power-marker migration: {e}"))?;
    let mut session = crate::Session::default();
    session.open_snapshot(original);
    let mut replacements = BTreeMap::new();
    for (path, mut marker, definition) in markers {
        marker["style"] = json!("global");
        let marker: crate::net_labels::Label = serde_json::from_value(marker).map_err(|e| e.to_string())?;
        let sheet = path.strip_prefix("sheets/").and_then(|p| p.strip_suffix(".json")).ok_or("invalid sheet path")?;
        let definition = session.libraries().symbol("power", definition, 1).ok_or("missing bundled power definition revision 1")?;
        session
            .apply(
                crate::PLACE_SYMBOL,
                &json!({
                    "sheet":sheet,"library":"power","definition":definition.id,
                    "definition_revision":definition.revision(),"definition_digest":crate::library::sources::symbol_digest(&definition)?,
                    "x_nm":marker.at.x_nm,"y_nm":marker.at.y_nm,"rotation":marker.rotation,
                    "value":marker.name,"auto_heal":false,"expected_revision":session.revision()
                }),
            )
            .map_err(|e| e.to_string())?;
        let snapshot = session.snapshot().ok_or("missing migrated project")?;
        let target = crate::SheetTarget { project: snapshot.id(), sheet: Uuid::parse_str(sheet).map_err(|e| e.to_string())? };
        let own = snapshot.for_sheet(target).ok_or("missing migrated sheet")?;
        let symbol = own.symbols().last().ok_or("missing placed power symbol")?;
        replacements.insert((path, marker.id), (symbol.ordinal, marker.text));
    }
    let mut encoded = encode_documents(session.snapshot().ok_or("missing migrated snapshot")?)?;
    let changed_sheets: BTreeSet<_> = replacements.keys().map(|(path, _)| path.clone()).collect();
    for path in &changed_sheets {
        let mut sheet: Value = serde_json::from_slice(&encoded[path]).map_err(|e| e.to_string())?;
        let original: Value = serde_json::from_slice(&documents[path]).map_err(|e| e.to_string())?;
        sheet["wires"] = original["wires"].clone();
        sheet["junctions"] = original["junctions"].clone();
        if let Some(labels) = sheet["net_labels"].as_array_mut() {
            labels.retain(|label| !replacements.contains_key(&(path.clone(), label["id"].as_u64().unwrap_or(0))));
        }
        if let Some(wires) = sheet["wires"].as_array_mut() {
            for wire in wires {
                for end in ["from", "to"] {
                    if let Some(id) = wire[end]["net_label"].as_u64()
                        && let Some((symbol, _)) = replacements.get(&(path.clone(), id))
                    {
                        wire[end] = json!({"symbol":symbol,"pin":"1"});
                    }
                }
            }
        }
        let layouts: BTreeMap<_, _> = replacements.iter().filter(|((owner, _), _)| owner == path).map(|(_, (id, text))| (*id, text)).collect();
        if let Some(symbols) = sheet["symbols"].as_array_mut() {
            for symbol in symbols {
                if let Some(id) = symbol["id"].as_u64()
                    && let Some(Some(text)) = layouts.get(&id)
                {
                    symbol["field_layout"]["value"] = serde_json::to_value(text).map_err(|e| e.to_string())?;
                }
            }
        }
        encoded.insert(path.clone(), serde_json::to_vec_pretty(&sheet).map_err(|e| e.to_string())?);
    }
    for (path, bytes) in encoded {
        if path == ENTRY_POINT || changed_sheets.contains(&path) || !documents.contains_key(&path) {
            documents.insert(path, bytes);
        }
    }
    decode_documents(&documents).map_err(|e| format!("After power-marker migration: {e}"))?;
    Ok(documents)
}

/// A JSON tree that refuses duplicate object keys at every depth before any sheet rewrite.
/// Typed document validation still checks the sanitized schema and all cross-document references.
struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    /// Preserve JSON values while rejecting repeated fields instead of retaining the last one.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> serde::de::Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON with unique object fields")
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }

            /// JSON numbers must remain finite and representable by the normal JSON value model.
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value).map(Value::Number).map(UniqueValue).ok_or_else(|| E::custom("invalid JSON number"))
            }

            /// Check objects nested anywhere inside an array before retaining the array.
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }

            /// Reject a duplicate before reading or replacing its value, including nested records.
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!("duplicate field `{key}`")));
                    }
                    let UniqueValue(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Repeated fields at the sheet, marker or endpoint level cannot be normalized away.
    /// Normal package Open must reject them before an active project could be replaced.
    #[test]
    fn duplicate_fields_are_rejected_before_marker_rewriting() {
        let mut session = crate::Session::default();
        session.open_snapshot(decode(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap());
        let at = session.project().unwrap().symbols()[0].pins()[0];
        session
            .apply(
                crate::net_labels::ADD,
                &json!({"name":"SUPPLY","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":session.revision()}),
            )
            .unwrap();
        let marker = session.project().unwrap().net_labels()[0].id;
        let mut documents = encode_documents(session.snapshot().unwrap()).unwrap();
        let path = format!("sheets/{}.json", session.project().unwrap().sheet_target().sheet);
        let mut sheet: Value = serde_json::from_slice(&documents[&path]).unwrap();
        sheet["net_labels"][0]["style"] = json!("power");
        sheet["wires"][0]["from"] = json!({"net_label":marker});
        let text = serde_json::to_string(&sheet).unwrap();
        let repeated_wires = format!("{},\"wires\":[]}}", &text[..text.len() - 1]);
        let repeated_name = text.replacen("\"name\":\"SUPPLY\"", "\"name\":\"SUPPLY\",\"name\":\"OTHER\"", 1);
        let endpoint = format!("{{\"net_label\":{marker}}}");
        let repeated_endpoint = text.replacen(&endpoint, &format!("{{\"net_label\":{marker},\"net_label\":{marker}}}"), 1);
        for (field, malformed) in [("wires", repeated_wires), ("name", repeated_name), ("net_label", repeated_endpoint)] {
            assert_ne!(malformed, text);
            documents.insert(path.clone(), malformed.into_bytes());
            let error = migrate_power_markers(documents.clone()).unwrap_err();
            assert!(error.contains(&path) && error.contains("duplicate field") && error.contains(field), "{error}");
            let packed = encode_packed_documents(documents.clone()).unwrap();
            assert!(decode_packed(&packed).err().unwrap().contains("duplicate field"));
        }
    }

    /// Every retired drawing becomes a pinned native symbol with the same named contact,
    /// rotation and wire route; unrelated payloads survive and conversion is deterministic.
    #[test]
    fn retired_markers_preserve_contacts_routes_and_payloads() {
        for (style, definition) in [("power", "vcc"), ("power_bar", "vcc-bar"), ("ground", "gnd")] {
            let mut session = crate::Session::default();
            session.open_snapshot(decode(include_bytes!("../../../app/tests/fixtures/two-resistors.pc")).unwrap());
            let resistor = session.project().unwrap().symbols()[0].clone();
            let at = resistor.pins()[0];
            session
                .apply(
                    crate::net_labels::ADD,
                    &json!({"name":"SUPPLY","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":90,"expected_revision":session.revision()}),
                )
                .unwrap();
            let marker = session.project().unwrap().net_labels()[0].id;
            let mut old = encode_documents(session.snapshot().unwrap()).unwrap();
            let path = format!("sheets/{}.json", session.project().unwrap().sheet_target().sheet);
            let mut sheet: Value = serde_json::from_slice(&old[&path]).unwrap();
            sheet["net_labels"][0]["style"] = json!(style);
            sheet["wires"][0]["from"] = json!({"net_label":marker});
            old.insert(path.clone(), serde_json::to_vec(&sheet).unwrap());
            let migrated = migrate_power_markers(old.clone()).unwrap();
            assert_eq!(migrate_power_markers(old.clone()).unwrap(), migrated);
            assert_eq!(migrate_power_markers(migrated.clone()).unwrap(), migrated);
            let package = encode_packed_documents(old.clone()).unwrap();
            let opened = decode_packed(&package).unwrap();
            assert!(opened.needs_manifest, "an unchanged Save must persist semantic migration");
            assert_eq!(opened.documents, migrated);
            assert!(!decode_packed(&encode_packed_documents(opened.documents).unwrap()).unwrap().needs_manifest);
            let project = decode_documents(&migrated).unwrap();
            assert!(project.net_labels().is_empty());
            let power = project.symbols().iter().find(|s| s.definition.id == definition).unwrap();
            assert_eq!(power.position, at);
            assert_eq!(power.rotation, 1);
            assert_eq!(project.value(power), "SUPPLY");
            let graph = project.electrical_connectivity();
            let terminal = power.terminals()[0];
            assert_eq!(graph.net_name(graph.net_index_for(terminal).unwrap()), "SUPPLY");
            assert!(graph.net_for(terminal).unwrap().len() >= 2);
            assert!(project.component(power).unwrap().metadata.attributes.exclude_from_board);
            let converted: Value = serde_json::from_slice(&migrated[&path]).unwrap();
            assert_eq!(converted["junctions"], sheet["junctions"]);
            assert_eq!(converted["wires"][0]["id"], sheet["wires"][0]["id"]);
            assert_eq!(converted["wires"][0]["bends"], sheet["wires"][0]["bends"]);
            assert_eq!(converted["wires"][0]["from"], json!({"symbol":power.ordinal,"pin":"1"}));
            for (other, bytes) in &old {
                if other != ENTRY_POINT && other != &path {
                    assert_eq!(&migrated[other], bytes);
                }
            }
            sheet["net_labels"][0]["unknown"] = json!(true);
            old.insert(path.clone(), serde_json::to_vec(&sheet).unwrap());
            assert!(migrate_power_markers(old).is_err());
        }
    }
}
