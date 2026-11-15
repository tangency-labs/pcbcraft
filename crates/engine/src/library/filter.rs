//! Optional placement discovery criteria; these never validate or mutate a project.
use super::parts::{Category, Requirement};
use crate::part_instances::Selection;
use serde::{Deserialize, Serialize};

/// A required parameter can demand presence alone or use the existing typed comparisons.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<Requirement>,
}
/// One category scope keeps the placement filter small and its field meanings unambiguous.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Filter {
    pub enabled: bool,
    pub manufacturer: bool,
    pub footprint: bool,
    pub category: Option<Category>,
    pub parameters: Vec<Parameter>,
}
impl Filter {
    /// Bound saved preferences and validate field comparisons without requiring source availability.
    pub fn validate(&self) -> Result<(), String> {
        if self.parameters.len() > 32 {
            return Err("At most 32 parameter filters are supported.".into());
        }
        if let Some(category) = &self.category {
            category.validate()?;
        }
        if !self.parameters.is_empty() && self.category.is_none() {
            return Err("Choose a category before adding parameter filters.".into());
        }
        let mut fields = std::collections::BTreeSet::new();
        for p in &self.parameters {
            if !fields.insert(&p.field) {
                return Err("Duplicate parameter filter.".into());
            }
            let field = self.category.as_ref().and_then(|c| c.fields.get(&p.field)).ok_or("Parameter is absent from the selected category.")?;
            if let Some(r) = &p.requirement {
                r.validate(field)?;
            }
        }
        Ok(())
    }
    /// Report whether the configured filter actually narrows choices.
    pub fn active(&self) -> bool {
        self.enabled && (self.manufacturer || self.footprint || self.category.is_some())
    }
    /// Match the captured choice using canonical data. An orderable part's missing specification
    /// remains unknown; it never falls back to a generic assumption about that actual product.
    pub fn matches(&self, selection: &Selection) -> bool {
        if selection.resolved.symbol.as_ref().is_some_and(|s| s.power_terminal().is_some()) {
            return true;
        }
        if !self.active() {
            return true;
        }
        if self.manufacturer && selection.manufacturer.is_none() {
            return false;
        }
        if self.footprint && selection.package.as_ref().is_none_or(|id| !selection.resolved.footprints.contains_key(id)) {
            return false;
        }
        if let Some(category) = &self.category {
            if selection.resolved.category.as_deref() != Some(category) {
                return false;
            }
            for p in &self.parameters {
                let value = if let Some(manufacturer) = &selection.manufacturer {
                    manufacturer.specifications.get(&p.field)
                } else {
                    selection.resolved.definition.fixed_parameters.get(&p.field).or_else(|| match selection.requirements.get(&p.field) {
                        Some(Requirement::Exact { value }) => Some(value),
                        _ => None,
                    })
                };
                if value.is_none_or(|v| p.requirement.as_ref().is_some_and(|r| !r.matches(v))) {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{
        parts::{Reference, Value},
        sources::{Catalog, Documents, NativeSource},
    };
    use std::sync::Arc;
    /// Load the original native fixture with exact typed category and manufacturer facts.
    fn selection() -> Selection {
        let docs: Documents = [
            ("library.json", include_str!("../../../../examples/native-parts/library.json")),
            ("symbols/resistor.json", include_str!("../../../../examples/native-parts/symbols/resistor.json")),
            ("footprints/resistor.json", include_str!("../../../../examples/native-parts/footprints/resistor-0603.json")),
            ("categories/resistor.json", include_str!("../../../../examples/native-parts/categories/resistor.json")),
            ("parts/resistor.json", include_str!("../../../../examples/native-parts/parts/resistor.json")),
            ("manufacturer-parts/resistor.json", include_str!("../../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json")),
        ]
        .into_iter()
        .map(|(p, b)| (p.into(), b.into()))
        .collect();
        let mut catalog = Catalog::default();
        catalog.install("fixture", Arc::new(NativeSource::load(&docs).unwrap())).unwrap();
        let reference = Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
        Selection {
            resolved: Arc::new(catalog.resolve_part(&reference).unwrap().clone()),
            package: Some("0603".into()),
            manufacturer: catalog.manufacturers_for(&reference).pop(),
            requirements: Default::default(),
        }
    }
    /// Manufacturer/package presence and canonical range comparisons narrow choices independently.
    #[test]
    fn placement_filter_matches_typed_facts_and_missing_data_fails() {
        let mut part = selection();
        let mut f = Filter {
            enabled: true,
            manufacturer: true,
            footprint: true,
            category: part.resolved.category.as_deref().cloned(),
            parameters: vec![Parameter {
                field: "pcbcraft:resistance".into(),
                requirement: Some(Requirement::Range {
                    minimum: Value::Quantity { value: "0.09".into(), unit: "Ohm".into() },
                    maximum: Value::Quantity { value: "0.11".into(), unit: "Ohm".into() },
                }),
            }],
        };
        f.validate().unwrap();
        assert!(f.matches(&part));
        part.package = None;
        assert!(!f.matches(&part));
        part.package = Some("0603".into());
        Arc::make_mut(part.manufacturer.as_mut().unwrap()).specifications.remove("pcbcraft:resistance");
        assert!(!f.matches(&part));
        part.manufacturer = None;
        assert!(!f.matches(&part));
        f.enabled = false;
        assert!(f.matches(&part));
    }
    /// A category owns the type/unit and optional field meanings; invalid or duplicate rules fail.
    #[test]
    fn invalid_filter_criteria_are_not_accepted() {
        let part = selection();
        let mut f = Filter {
            category: part.resolved.category.as_deref().cloned(),
            parameters: vec![Parameter {
                field: "pcbcraft:resistance".into(),
                requirement: Some(Requirement::Exact { value: Value::Quantity { value: "1".into(), unit: "V".into() } }),
            }],
            ..Default::default()
        };
        assert!(f.validate().is_err());
        f.parameters[0].requirement = None;
        f.validate().unwrap();
        f.parameters.push(f.parameters[0].clone());
        assert!(f.validate().is_err());
    }
}
