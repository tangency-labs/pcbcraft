//! Typed field schemas and explicit category bindings.
use super::{
    CATEGORY_FORMAT, Quantity, Reference,
    codec::{decode, encode, header, unique_map},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
/// Explicit category/Value binding. Omission leaves a symbol or part uncategorized, never inferred.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryBinding {
    pub definition: Reference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_value: Option<String>,
}
impl CategoryBinding {
    /// Validate the address and optional namespaced field without fetching its category.
    pub fn validate(&self) -> Result<(), String> {
        self.definition.validate()?;
        if self.primary_value.as_deref().is_some_and(|id| !field_id(id)) {
            return Err("Primary value must name a namespaced category field.".into());
        }
        Ok(())
    }
    /// Require the exact category and a quantity field for the unit-aware Value control.
    pub fn validate_category(&self, category: &Category) -> Result<(), String> {
        self.validate()?;
        category.validate()?;
        if self.definition != category.reference() {
            return Err("Category identity/revision does not match the binding.".into());
        }
        if let Some(id) = &self.primary_value
            && !matches!(category.fields.get(id).map(|field| &field.value_type), Some(FieldType::Quantity { .. }))
        {
            return Err(format!("Primary value {id} must name a quantity field in the category."));
        }
        Ok(())
    }
}

/// A field's type is pinned with the category, independently of its editable display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldType {
    Quantity { quantity: Quantity, unit: String },
    Integer { minimum: i64, maximum: i64 },
    Choice { choices: Vec<String> },
    Text {},
    Boolean {},
}
impl FieldType {
    /// Shared field identities retain type/quantity/unit meaning; category-specific bounds and
    /// permitted choices may differ without changing how an existing value is interpreted.
    pub fn same_meaning(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Quantity { quantity, unit }, Self::Quantity { quantity: other_quantity, unit: other_unit }) => {
                quantity == other_quantity && unit == other_unit
            }
            (Self::Integer { .. }, Self::Integer { .. })
            | (Self::Choice { .. }, Self::Choice { .. })
            | (Self::Text {}, Self::Text {})
            | (Self::Boolean {}, Self::Boolean {}) => true,
            _ => false,
        }
    }
}

/// Typed category field; namespaced map keys are its stable identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub value_type: FieldType,
}

/// A category defines fields and units, never part-instance values or manufacturer specifications.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub format: String,
    pub library: String,
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub license: String,
    #[serde(deserialize_with = "unique_map")]
    pub fields: BTreeMap<String, Field>,
}
impl Category {
    /// Read a bounded standalone category with strict fields and semantic type/unit validation.
    pub fn read(text: &str) -> Result<Self, String> {
        let category: Self = decode(text)?;
        category.validate()?;
        Ok(category)
    }
    /// Its permanent exact address, independent of where a source stores the document.
    pub fn reference(&self) -> Reference {
        Reference { library: self.library.clone(), id: self.id.clone(), revision: self.revision }
    }
    /// Validate field identity/type, canonical units and bounded, distinct enumeration choices.
    pub fn validate(&self) -> Result<(), String> {
        header(&self.format, CATEGORY_FORMAT, &self.reference(), &self.name, &self.license)?;
        if self.fields.is_empty() || self.fields.len() > 64 {
            return Err("A category requires 1–64 fields.".into());
        }
        for (id, field) in &self.fields {
            if !field_id(id) {
                return Err(format!("Invalid namespaced category field {id:?}."));
            }
            crate::library::check_text("field name", &field.name, 64)?;
            match &field.value_type {
                FieldType::Quantity { quantity, unit } if unit != quantity.unit() => {
                    return Err(format!("Field {id} requires canonical unit {}.", quantity.unit()));
                }
                FieldType::Choice { choices } => {
                    if choices.is_empty() || choices.len() > 64 || choices.iter().collect::<BTreeSet<_>>().len() != choices.len() {
                        return Err(format!("Field {id} requires 1–64 distinct choices."));
                    }
                    for choice in choices {
                        crate::library::check_text("choice", choice, 64)?;
                    }
                }
                FieldType::Integer { minimum, maximum }
                    if minimum > maximum || minimum.unsigned_abs() > 9_007_199_254_740_991 || maximum.unsigned_abs() > 9_007_199_254_740_991 =>
                {
                    return Err(format!("Field {id} requires ordered, JSON-safe integer bounds."));
                }
                _ => {}
            }
        }
        Ok(())
    }
    /// Serialize validated content deterministically, with sorted field identities and no caches.
    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        encode(self)
    }
}

/// Field IDs are namespace:item, with portable independently validated segments.
pub fn field_id(id: &str) -> bool {
    id.split_once(':').is_some_and(|(namespace, field)| crate::library::check_id(namespace).is_ok() && crate::library::check_id(field).is_ok())
}

impl Field {
    /// Parse editable field input into its canonical typed value; display labels never determine units.
    pub fn parse_input(&self, input: &str) -> Result<super::Value, String> {
        let input = input.trim();
        let value = match &self.value_type {
            FieldType::Quantity { quantity, .. } => quantity.parse(input)?,
            FieldType::Integer { .. } => super::Value::Integer { value: input.parse().map_err(|_| "Expected an integer.")? },
            FieldType::Choice { .. } => super::Value::Choice { value: input.into() },
            FieldType::Text {} => super::Value::Text { value: input.into() },
            FieldType::Boolean {} => super::Value::Boolean { value: input.parse().map_err(|_| "Expected true or false.")? },
        };
        value.validate_field(self)?;
        Ok(value)
    }
}
