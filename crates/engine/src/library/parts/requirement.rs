//! Typed design requirements are compared with manufacturer specifications without parsing display text.
use super::{Field, Value};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// A requirement belongs to an instance; fixed generic-part values remain exact requirements.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operator", rename_all = "snake_case", deny_unknown_fields)]
pub enum Requirement {
    Exact { value: Value },
    AtLeast { value: Value },
    AtMost { value: Value },
    Range { minimum: Value, maximum: Value },
    OneOf { values: Vec<Value> },
}
impl Requirement {
    /// Validate values against the pinned category, including numeric ordering and bounded choices.
    pub fn validate(&self, field: &Field) -> Result<(), String> {
        match self {
            Self::Exact { value } => value.validate_field(field),
            Self::AtLeast { value } | Self::AtMost { value } => {
                value.validate_field(field)?;
                compare(value, value).ok_or("Ordered requirements require numeric quantities or integers.")?;
                Ok(())
            }
            Self::Range { minimum, maximum } => {
                minimum.validate_field(field)?;
                maximum.validate_field(field)?;
                if compare(minimum, maximum).is_none_or(|order| order == Ordering::Greater) {
                    return Err("Range requires ordered compatible numeric values.".into());
                }
                Ok(())
            }
            Self::OneOf { values } => {
                if values.is_empty() || values.len() > 64 {
                    return Err("One-of requires 1–64 values.".into());
                }
                for (index, value) in values.iter().enumerate() {
                    value.validate_field(field)?;
                    if values[..index].contains(value) {
                        return Err("One-of repeats a value.".into());
                    }
                }
                Ok(())
            }
        }
    }
    /// Missing specifications do not satisfy a requirement; no conversions or guesses occur here.
    pub fn matches(&self, specification: &Value) -> bool {
        match self {
            Self::Exact { value } => value == specification,
            Self::AtLeast { value } => compare(specification, value).is_some_and(|order| order != Ordering::Less),
            Self::AtMost { value } => compare(specification, value).is_some_and(|order| order != Ordering::Greater),
            Self::Range { minimum, maximum } => {
                compare(specification, minimum).is_some_and(|order| order != Ordering::Less)
                    && compare(specification, maximum).is_some_and(|order| order != Ordering::Greater)
            }
            Self::OneOf { values } => values.contains(specification),
        }
    }
}
/// Compare canonical values at full precision. Different quantities must already share a category field.
fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Integer { value: a }, Value::Integer { value: b }) => Some(a.cmp(b)),
        (Value::Quantity { value: a, unit: ua }, Value::Quantity { value: b, unit: ub }) if ua == ub => {
            if !super::decimal(a) || !super::decimal(b) {
                return None;
            }
            let (na, nb) = (a.starts_with('-'), b.starts_with('-'));
            if na != nb {
                return Some(if na { Ordering::Less } else { Ordering::Greater });
            }
            let (wa, fa) = a.trim_start_matches('-').split_once('.').unwrap_or((a.trim_start_matches('-'), ""));
            let (wb, fb) = b.trim_start_matches('-').split_once('.').unwrap_or((b.trim_start_matches('-'), ""));
            let order = wa
                .len()
                .cmp(&wb.len())
                .then_with(|| wa.cmp(wb))
                .then_with(|| fa.bytes().chain(std::iter::repeat(b'0')).take(18).cmp(fb.bytes().chain(std::iter::repeat(b'0')).take(18)));
            Some(if na { order.reverse() } else { order })
        }
        _ => None,
    }
}

/// Publish the shared command envelope; category-dependent meaning is validated after resolution.
pub fn requirements_schema() -> serde_json::Value {
    use serde_json::json;
    let value = json!({"type":"object","required":["type","value"],"properties":{"type":{"enum":["quantity","integer","choice","text","boolean"]},"value":{},"unit":{"type":"string"}},"additionalProperties":false});
    let branches = [
        json!({"type":"object","required":["operator","value"],"properties":{"operator":{"enum":["exact","at_least","at_most"]},"value":value},"additionalProperties":false}),
        json!({"type":"object","required":["operator","minimum","maximum"],"properties":{"operator":{"const":"range"},"minimum":value,"maximum":value},"additionalProperties":false}),
        json!({"type":"object","required":["operator","values"],"properties":{"operator":{"const":"one_of"},"values":{"type":"array","minItems":1,"maxItems":64,"uniqueItems":true,"items":value}},"additionalProperties":false}),
    ];
    json!({"type":"object","maxProperties":64,"additionalProperties":{"oneOf":branches}})
}
