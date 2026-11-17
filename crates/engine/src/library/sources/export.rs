//! Whole-library export reuses the original native documents of one loaded source.
use super::Catalog;
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
    /// other libraries are never collected: dependencies on them are reported, and a dependency
    /// that does not resolve through the catalog's resolver is reported as unavailable.
    pub fn export_bundle(&self, source: &str) -> Result<Export, String> {
        let native = self.sources.get(source).filter(|_| self.source_available(source)).ok_or("Choose an available library to export.")?;
        let mut external = BTreeSet::new();
        let mut unavailable = Vec::new();
        for item in &native.items {
            let mut libraries = Vec::new();
            let resolved = if let Some(binding) = item.symbol().and_then(|symbol| symbol.category.clone()) {
                self.category(&binding.definition).map(|category| libraries.push(category.library.clone()))
            } else if let Some(reference) = item.part().map(|part| part.reference()).or_else(|| item.manufacturer_part().map(|m| m.part.clone())) {
                self.resolve_part(&reference).map(|part| {
                    libraries.push(part.definition.library.clone());
                    libraries.extend(part.symbol.iter().map(|symbol| symbol.library.clone()));
                    libraries.extend(part.category.iter().map(|category| category.library.clone()));
                    libraries.extend(part.footprints.values().map(|footprint| footprint.library.clone()));
                })
            } else {
                Ok(())
            };
            match resolved {
                Ok(()) => external.extend(libraries.into_iter().filter(|library| *library != native.library)),
                Err(error) => unavailable.push(format!("{}: {error}", item.file)),
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
        install(&mut catalog, "demo", &documents("footprints/resistor-0603.json"));
        let report = catalog.export_bundle("demo").unwrap().report;
        assert!(report.contains("Unavailable dependencies: parts/resistor.json:"), "{report}");
    }
}
