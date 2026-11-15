//! Reusable generic parts link schematic, category and physical package definitions.
use super::{
    Category, CategoryBinding, LEGACY_PART_FORMAT, PART_FORMAT, Package, Reference, Value,
    codec::{decode, encode, header, unique_map},
    field_id,
};
use crate::board::definition::Source;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// A reusable generic part contains no drawings, manufacturer identity, stock or prices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartDefinition {
    pub format: String,
    pub library: String,
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub license: String,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "optional_symbol")]
    pub symbol: Option<Reference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<CategoryBinding>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "unique_map")]
    pub fixed_parameters: BTreeMap<String, Value>,
    #[serde(deserialize_with = "unique_map")]
    pub packages: BTreeMap<String, Package>,
}
impl PartDefinition {
    /// Parse an actual native part file; unresolved dependencies are checked separately by the catalog.
    pub fn read(text: &str) -> Result<Self, String> {
        let part: Self = decode(text)?;
        part.validate()?;
        Ok(part)
    }
    /// Its immutable address, independent of source registrations and paths.
    pub fn reference(&self) -> Reference {
        Reference { library: self.library.clone(), id: self.id.clone(), revision: self.revision }
    }
    /// Validate closed records and local invariants; empty package/category choices remain valid.
    pub fn validate(&self) -> Result<(), String> {
        let expected = if self.format == LEGACY_PART_FORMAT { LEGACY_PART_FORMAT } else { PART_FORMAT };
        header(&self.format, expected, &self.reference(), &self.name, &self.license)?;
        if self.format == LEGACY_PART_FORMAT && self.symbol.is_none() {
            return Err("Format 1 parts require a symbol; use pcbcraft-part/2 for optional drawings.".into());
        }
        if let Some(symbol) = &self.symbol {
            symbol.validate()?;
        }
        for (name, value, limit) in [
            ("source kind", &self.source.kind, 64),
            ("source reference", &self.source.reference, 2048),
            ("source locator", &self.source.locator, 1024),
        ] {
            crate::library::check_text(name, value, limit)?;
        }
        if self.packages.len() > 64 || self.fixed_parameters.len() > 64 {
            return Err("A part supports at most 64 packages and 64 fixed parameters.".into());
        }
        if let Some(category) = &self.category {
            category.validate()?;
        } else if !self.fixed_parameters.is_empty() {
            return Err("Fixed parameters require an explicit category binding.".into());
        }
        for (id, value) in &self.fixed_parameters {
            if !field_id(id) {
                return Err(format!("Invalid namespaced parameter {id:?}."));
            }
            value.validate()?;
        }
        for (id, package) in &self.packages {
            crate::library::check_id(id)?;
            package.validate()?;
            if self.symbol.is_none() && (!package.mapping.is_empty() || !package.unconnected_pads.is_empty()) {
                return Err("A part without a symbol has no terminal map; leave mappings and unconnected pads empty.".into());
            }
            if self.symbol.is_some() && package.mapping.is_empty() {
                return Err("A package with a symbol requires a terminal mapping.".into());
            }
        }
        Ok(())
    }
    /// Resolve fixed requirements against one exact category; never treat untyped values as quantities.
    pub fn validate_category(&self, category: Option<&Category>) -> Result<(), String> {
        self.validate()?;
        match (&self.category, category) {
            (Some(binding), Some(category)) => {
                binding.validate_category(category)?;
                for (id, value) in &self.fixed_parameters {
                    let field = category.fields.get(id).ok_or_else(|| format!("Unknown category field {id}."))?;
                    value.validate_field(field).map_err(|e| format!("{id}: {e}"))?;
                }
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err("Part category is missing or was supplied without a binding.".into()),
        }
    }
    /// Encode a validated standalone record with stable map order and no runtime/source cache data.
    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        encode(self)
    }
}

/// An absent association is optional; explicit null is not an exact reference.
fn optional_symbol<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Reference>, D::Error> {
    Reference::deserialize(deserializer).map(Some)
}
