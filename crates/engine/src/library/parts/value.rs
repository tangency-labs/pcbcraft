//! Canonical typed field values validated against category field meanings.
use super::{Field, FieldType, display_quantity};
use pcbcraft_units::quantity::{CanonicalQuantity, validate_quantity};
use serde::{Deserialize, Serialize};
/// Known fixed requirements of a generic part. Unknown/missing specifications are not fixed values.
/// Decimal strings are canonical base-unit values, preserving precision across JSON consumers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Value {
    Quantity { value: String, unit: String },
    Integer { value: i64 },
    Choice { value: String },
    Text { value: String },
    Boolean { value: bool },
}
impl Value {
    /// Bound values before category resolution; quantities use plain canonical decimal notation.
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Quantity { value, unit } => {
                validate_quantity(value, unit)?;
            }
            Self::Choice { value } => crate::library::check_text("choice value", value, 64)?,
            Self::Integer { value } if value.unsigned_abs() > 9_007_199_254_740_991 => {
                return Err("Integer must be exactly representable by JSON consumers.".into());
            }
            Self::Text { value } => crate::library::check_text("text value", value, 128)?,
            Self::Boolean { .. } | Self::Integer { .. } => {}
        }
        Ok(())
    }
    /// Readable presentation text; quantities use SI prefixes via [`display_quantity`] and parse back exactly.
    pub fn display(&self) -> String {
        match self {
            Self::Quantity { value, unit } => display_quantity(value, unit),
            Self::Integer { value } => value.to_string(),
            Self::Choice { value } | Self::Text { value } => value.clone(),
            Self::Boolean { value } => value.to_string(),
        }
    }
    /// Check meaning against the pinned category, never the field's display label.
    pub fn validate_field(&self, field: &Field) -> Result<(), String> {
        self.validate()?;
        let valid = match (self, &field.value_type) {
            (Self::Quantity { unit, .. }, FieldType::Quantity { unit: expected, .. }) => unit == expected,
            (Self::Choice { value }, FieldType::Choice { choices }) => choices.contains(value),
            (Self::Integer { value }, FieldType::Integer { minimum, maximum }) => (*minimum..=*maximum).contains(value),
            (Self::Text { .. }, FieldType::Text {}) | (Self::Boolean { .. }, FieldType::Boolean {}) => true,
            _ => false,
        };
        if valid { Ok(()) } else { Err("Value type, unit or choice does not match its category field.".into()) }
    }
}

/// Adapt a shared canonical quantity to the existing native typed value vocabulary.
impl From<CanonicalQuantity> for Value {
    /// Preserve exact strings and the existing quantity tag at the host boundary.
    fn from(quantity: CanonicalQuantity) -> Self {
        let (value, unit) = quantity.into_parts();
        Self::Quantity { value, unit }
    }
}
