//! Whole-library export reuses the original native documents of one loaded source.
use super::{Catalog, Key, Kind, MAX_SOURCE_BYTES, Reference};
use serde_json::json;
use std::collections::BTreeSet;

/// A `pcbcraft-library-bundle/1` file for one source and what its recipient should know.
#[derive(Debug)]
pub struct Export {
    pub file_name: String,
    pub bytes: Vec<u8>,
    /// Names the library and reports other libraries it depends on but does not include.
    pub report: String,
}

impl Catalog {
    /// Bundle the original documents of one available source without changing it. Records from
    /// other libraries are never collected: dependencies on them are reported, and each declared
    /// dependency that isn't available in the catalog is reported. A bundle larger than Add
    /// library accepts is an error.
    pub fn export_bundle(&self, source: &str) -> Result<Export, String> {
        let native = self.sources.get(source).filter(|_| self.source_available(source)).ok_or("Choose an available library to export.")?;
        let mut external = BTreeSet::new();
        let mut unavailable = Vec::new();
        let own: BTreeSet<&Key> = native.items.iter().map(|item| &item.key).collect();
        for item in &native.items {
            // Check each declared reference on its own, so one missing dependency can't hide the others.
            let mut references: Vec<(Reference, Kind)> = Vec::new();
            if let Some(binding) = item.symbol().and_then(|symbol| symbol.category.clone()) {
                references.push((binding.definition, Kind::Category));
            }
            if let Some(part) = item.part() {
                references.extend(part.symbol.iter().map(|symbol| (symbol.clone(), Kind::Symbol)));
                references.extend(part.category.iter().map(|binding| (binding.definition.clone(), Kind::Category)));
                references.extend(part.packages.values().map(|package| (package.footprint.clone(), Kind::Footprint)));
            }
            if let Some(record) = item.manufacturer_part() {
                references.push((record.part.clone(), Kind::Part));
            }
            for (reference, kind) in references {
                let key = Key { library: reference.library.clone(), kind, id: reference.id.clone(), revision: reference.revision };
                if reference.library != native.library {
                    external.insert(reference.library.clone());
                } else if !own.contains(&key)
                    && let Some(other) = self.index.get(&key).and_then(|(source, _)| self.sources.get(source))
                {
                    // Same library ID, but another loaded source supplies the record.
                    external.insert(other.name.clone());
                }
                // A category is only usable when its field definitions don't conflict, too.
                let found = if kind == Kind::Category { self.category(&reference).map(drop) } else { self.exact(&reference, kind).map(drop) };
                if let Err(error) = found {
                    unavailable.push(format!("{}: {error}", item.file));
                }
            }
        }
        let mut report = format!("Exported {} ({} items).", native.name, native.items.len());
        if !external.is_empty() {
            report += &format!(" Not included, install separately: {}.", external.into_iter().collect::<Vec<_>>().join(", "));
        }
        if !unavailable.is_empty() {
            report += &format!(" Unavailable dependencies: {}", unavailable[..unavailable.len().min(3)].join("; "));
            if unavailable.len() > 3 {
                report += &format!("; and {} more", unavailable.len() - 3);
            }
            report += ".";
        }
        if !native.problems.is_empty() {
            report += &format!(" The library reports {} file problems; review them before sharing.", native.problems.len());
        }
        let bundle = json!({"format": "pcbcraft-library-bundle/1", "documents": native.documents});
        let mut bytes = serde_json::to_vec_pretty(&bundle).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        // Escaping can grow the text past what Add library accepts; refuse rather than write it.
        if bytes.len() > MAX_SOURCE_BYTES * 2 {
            return Err(format!("This library's bundle would exceed {} MiB, the most Add library accepts.", MAX_SOURCE_BYTES * 2 / (1 << 20)));
        }
        Ok(Export { file_name: format!("{}.library.json", native.library), bytes, report })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Documents, NativeSource};
    use super::*;
    use std::sync::Arc;

    /// The original native parts example, optionally without one of its files.
    fn documents(without: &str) -> Documents {
        [
            ("library.json", include_str!("../../../../../examples/native-parts/library.json")),
            ("symbols/resistor.json", include_str!("../../../../../examples/native-parts/symbols/resistor.json")),
            ("footprints/resistor-0603.json", include_str!("../../../../../examples/native-parts/footprints/resistor-0603.json")),
            ("categories/resistor.json", include_str!("../../../../../examples/native-parts/categories/resistor.json")),
            ("parts/resistor.json", include_str!("../../../../../examples/native-parts/parts/resistor.json")),
        ]
        .into_iter()
        .filter(|(path, _)| *path != without)
        .map(|(path, text)| (path.into(), text.into()))
        .collect()
    }

    fn install(catalog: &mut Catalog, id: &str, documents: &Documents) {
        catalog.install(id, Arc::new(NativeSource::load(documents).unwrap())).unwrap();
    }

    /// The bundle carries the original text and installs back to the same items.
    #[test]
    fn export_round_trips_the_original_documents() {
        let mut catalog = Catalog::default();
        install(&mut catalog, "demo", &documents(""));
        let export = catalog.export_bundle("demo").unwrap();
        assert_eq!(export.file_name, "parts-demo.library.json");
        assert_eq!(NativeSource::bundle_documents(&export.bytes).unwrap(), documents(""));
        assert_eq!(NativeSource::from_bundle(&export.bytes).unwrap().items().len(), 4);
        assert_eq!(export.report, "Exported Native Parts Demo (4 items).");
        assert!(catalog.export_bundle("missing").is_err());
    }

    /// Another library's records are reported rather than collected, and so are dependencies
    /// the resolver cannot find.
    #[test]
    fn export_reports_other_and_unavailable_libraries() {
        let mut catalog = Catalog::default();
        install(&mut catalog, "demo", &documents(""));
        let mut record: serde_json::Value =
            serde_json::from_str(include_str!("../../../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json")).unwrap();
        record["library"] = "orders".into();
        let orders = Documents::from([
            ("library.json".into(), r#"{"format":"pcbcraft-library/1","id":"orders","name":"Orders","license":"MIT OR Apache-2.0"}"#.into()),
            ("manufacturer-parts/test-resistor-100m.json".into(), record.to_string()),
        ]);
        install(&mut catalog, "orders", &orders);
        let report = catalog.export_bundle("orders").unwrap().report;
        assert!(report.contains("install separately: parts-demo."), "{report}");
        catalog.remove("demo").unwrap();
        let report = catalog.export_bundle("orders").unwrap().report;
        assert!(report.contains("install separately: parts-demo. Unavailable dependencies: manufacturer-parts/"), "{report}");
        install(&mut catalog, "demo", &documents(""));
        install(&mut catalog, "demo", &documents("footprints/resistor-0603.json"));
        let report = catalog.export_bundle("demo").unwrap().report;
        assert!(report.contains("Unavailable dependencies: parts/resistor.json: Unavailable Footprint"), "{report}");
    }

    /// A category whose field meaning conflicts with another library's can't be used, so it is
    /// reported even though its record is present.
    #[test]
    fn export_reports_a_category_with_conflicting_fields() {
        let mut catalog = Catalog::default();
        let docs = documents("");
        install(&mut catalog, "demo", &docs);
        let mut category: serde_json::Value = serde_json::from_str(&docs["categories/resistor.json"]).unwrap();
        category["id"] = "incompatible-field".into();
        category["fields"]["pcbcraft:resistance"]["value_type"] = json!({"type": "quantity", "quantity": "voltage", "unit": "V"});
        let other = Documents::from([("library.json".into(), docs["library.json"].clone()), ("categories/other.json".into(), category.to_string())]);
        install(&mut catalog, "other", &other);
        let report = catalog.export_bundle("demo").unwrap().report;
        assert!(report.contains("parts/resistor.json: Conflicting type definitions for category field pcbcraft:resistance"), "{report}");
    }

    /// A record of the same library ID that another loaded source supplies isn't in the bundle,
    /// so that source is named too.
    #[test]
    fn export_names_another_source_of_the_same_library() {
        let mut catalog = Catalog::default();
        install(&mut catalog, "demo", &documents("footprints/resistor-0603.json"));
        let extra = Documents::from([
            (
                "library.json".into(),
                r#"{"format":"pcbcraft-library/1","id":"parts-demo","name":"Extra footprints","license":"MIT OR Apache-2.0"}"#.into(),
            ),
            ("footprints/resistor-0603.json".into(), documents("")["footprints/resistor-0603.json"].clone()),
        ]);
        install(&mut catalog, "extra", &extra);
        assert_eq!(
            catalog.export_bundle("demo").unwrap().report,
            "Exported Native Parts Demo (3 items). Not included, install separately: Extra footprints."
        );
    }

    /// Files that load as individual problems can still escape past the transport limit.
    #[test]
    fn export_refuses_a_bundle_add_library_would_reject() {
        let mut documents = documents("");
        for index in 0..12 {
            documents.insert(format!("symbols/padding-{index}.json"), "\0".repeat(super::super::MAX_FILE_BYTES));
        }
        let mut catalog = Catalog::default();
        install(&mut catalog, "demo", &documents);
        assert!(catalog.export_bundle("demo").unwrap_err().contains("most Add library accepts"));
    }
}
