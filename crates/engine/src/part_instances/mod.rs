//! Project-owned part selections reuse immutable library records and share one transaction authority.
pub(crate) mod commands;
pub(crate) mod storage;
use crate::library::{
    parts::{ManufacturerPart, Reference, Requirement, Value},
    sources::ResolvedPart,
};
pub use commands::{PLACE_PART, SET_PART};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, sync::Arc};

/// Project selection pins every required native record; source locations and provider state stay outside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub resolved: Arc<ResolvedPart>,
    pub package: Option<String>,
    pub manufacturer: Option<Arc<ManufacturerPart>>,
    pub requirements: BTreeMap<String, Requirement>,
}
/// Small persisted instance relationship; dependencies are separate deduplicated native documents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub definition: Reference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Reference>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "crate::library::parts::unique_map")]
    pub requirements: BTreeMap<String, Requirement>,
}
impl Selection {
    /// The record never contains paths, source registration names or mutable catalog snapshots.
    pub(crate) fn record(&self) -> Record {
        Record {
            definition: self.resolved.definition.reference(),
            package: self.package.clone(),
            manufacturer: self.manufacturer.as_ref().map(|m| m.reference()),
            requirements: self.requirements.clone(),
        }
    }
    /// Validate native closure and instance requirements; an edited MPN may remain recorded as incompatible.
    pub fn validate(&self) -> Result<(), String> {
        let part = &self.resolved.definition;
        part.validate_category(self.resolved.category.as_deref())?;
        if part.symbol
            != (Reference {
                library: self.resolved.symbol.library.clone(),
                id: self.resolved.symbol.id.clone(),
                revision: self.resolved.symbol.revision(),
            })
        {
            return Err("Pinned part symbol identity mismatch.".into());
        }
        if self.package.as_ref().is_some_and(|id| !part.packages.contains_key(id)) {
            return Err("Unavailable part package variant.".into());
        }
        if self.requirements.len() > 64 {
            return Err("A part instance supports at most 64 requirements.".into());
        }
        for (id, requirement) in &self.requirements {
            let field = self
                .resolved
                .category
                .as_ref()
                .and_then(|category| category.fields.get(id))
                .ok_or_else(|| format!("Unknown requirement field {id}."))?;
            requirement.validate(field)?;
            if let Some(fixed) = part.fixed_parameters.get(id)
                && requirement != &(Requirement::Exact { value: fixed.clone() })
            {
                return Err(format!("{id} is fixed by the reusable part."));
            }
        }
        if let Some(manufacturer) = &self.manufacturer {
            manufacturer.validate_part(&self.resolved)?;
        }
        Ok(())
    }
    /// Effective requirements include fixed values, even if the instance does not repeat them.
    pub fn effective_requirements(&self) -> BTreeMap<String, Requirement> {
        let mut values = self.requirements.clone();
        values.extend(self.resolved.definition.fixed_parameters.iter().map(|(id, value)| (id.clone(), Requirement::Exact { value: value.clone() })));
        values
    }
    /// Exact primary quantity as `(canonical decimal, base unit)`; a fixed parameter wins over a requirement.
    pub(crate) fn primary_quantity(&self) -> Option<(&str, &str)> {
        let field = self.resolved.definition.category.as_ref()?.primary_value.as_ref()?;
        let value = match self.resolved.definition.fixed_parameters.get(field) {
            Some(fixed) => fixed,
            None => match self.requirements.get(field)? {
                Requirement::Exact { value } => value,
                _ => return None,
            },
        };
        match value {
            Value::Quantity { value, unit } => Some((value, unit)),
            _ => None,
        }
    }
    /// Canonical primary quantity drives the legacy visible Value without changing stored units.
    pub fn primary_value(&self) -> Option<String> {
        self.primary_quantity().map(|(value, unit)| format!("{value} {unit}"))
    }
    /// SI-prefixed primary quantity for presentation only; [`Self::set_primary_value`] parses it back exactly.
    pub fn display_primary_value(&self) -> Option<String> {
        self.primary_quantity().map(|(value, unit)| crate::library::parts::display_quantity(value, unit))
    }
    /// Return a validated preview with the primary quantity normalized using its pinned category.
    pub fn with_primary_value(&self, input: &str) -> Result<Self, String> {
        let mut value = self.clone();
        if let Err(error) = value.set_primary_value(input)
            && (input != self.resolved.symbol.default_value() || self.primary_value().is_some() || self.manufacturer.is_some())
        {
            return Err(error);
        }
        value.validate()?;
        Ok(value)
    }
    /// Selecting a concrete product may supply an otherwise unspecified primary value.
    pub fn manufacturer_primary_default(&mut self) {
        if self.primary_value().is_some() {
            return;
        }
        if let Some(id) = self.resolved.definition.category.as_ref().and_then(|binding| binding.primary_value.as_ref())
            && !self.requirements.contains_key(id)
            && let Some(value) = self.manufacturer.as_ref().and_then(|record| record.specifications.get(id))
        {
            self.requirements.insert(id.clone(), Requirement::Exact { value: value.clone() });
        }
    }
    /// Normalize a visible Value edit against its explicit category; fixed values cannot be overridden.
    pub(crate) fn set_primary_value(&mut self, input: &str) -> Result<Option<String>, String> {
        let Some(id) = self.resolved.definition.category.as_ref().and_then(|binding| binding.primary_value.clone()) else {
            return Ok(None);
        };
        let field = self.resolved.category.as_ref().and_then(|category| category.fields.get(&id)).ok_or("Missing primary-value category field.")?;
        let crate::library::parts::FieldType::Quantity { quantity, .. } = &field.value_type else {
            return Err("Primary value must be a quantity.".into());
        };
        let value = quantity.parse(input)?;
        if let Some(fixed) = self.resolved.definition.fixed_parameters.get(&id) {
            if fixed != &value {
                return Err(format!("{id} is fixed by the reusable part."));
            }
            return Ok(self.primary_value());
        }
        self.requirements.insert(id, Requirement::Exact { value });
        self.validate()?;
        Ok(self.primary_value())
    }
    /// Inspect MPN compatibility without changing it, fetching specifications or selecting a substitute.
    pub fn compatibility(&self) -> Json {
        let Some(manufacturer) = &self.manufacturer else {
            return json!({"status":"unselected","findings":[]});
        };
        let mut findings = Vec::new();
        if self.package.as_deref() != Some(manufacturer.package.as_str()) {
            findings.push(json!({"field":"package","status":"does_not_meet","reason":"Selected package differs from the manufacturer's package."}));
        }
        for (id, requirement) in self.effective_requirements() {
            match manufacturer.specifications.get(&id) {
                None => findings.push(json!({"field":id,"status":"unknown","reason":"Manufacturer specification is unknown."})),
                Some(specification) if !requirement.matches(specification) => {
                    findings.push(json!({"field":id,"status":"does_not_meet","requirement":requirement,"specification":specification}))
                }
                _ => {}
            }
        }
        json!({"status":if findings.is_empty(){"meets_requirements"}else{"does_not_meet_requirements"},"findings":findings})
    }
    /// Public inspection uses pinned identities, effective typed requirements and supplied metadata.
    pub fn inspect(&self) -> Json {
        json!({"definition":self.resolved.definition.as_ref(),"category":self.resolved.category.as_deref(),"package":self.package,
            "requirements":self.effective_requirements(),"manufacturer":self.manufacturer.as_deref(),"compatibility":self.compatibility()})
    }
}

#[cfg(test)]
mod tests;
