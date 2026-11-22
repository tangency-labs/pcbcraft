//! Device-level operating supply limits on exact logical power-input terminals.
use super::{Quantity, Value, decimal};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Optional inclusive bounds in canonical volts. A missing bound is unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyRange {
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "bound")]
    pub supply_min: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "bound")]
    pub supply_max: Option<Value>,
}

/// A bound is omitted when unknown; explicit null is not a typed quantity.
fn bound<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

impl SupplyRange {
    /// Require at least one canonical voltage and an inclusive, non-reversed interval.
    pub(super) fn validate(&self, terminal: &str) -> Result<(), String> {
        let min = self.supply_min.as_ref().map(|v| voltage(terminal, "supply_min", v)).transpose()?;
        let max = self.supply_max.as_ref().map(|v| voltage(terminal, "supply_max", v)).transpose()?;
        match (min, max) {
            (None, None) => Err(format!("Terminal {terminal}: supply_min or supply_max is required.")),
            (Some(min), Some(max)) if compare_decimal(min, max).is_gt() => {
                Err(format!("Terminal {terminal}: supply_min {min} V exceeds supply_max {max} V."))
            }
            _ => Ok(()),
        }
    }

    /// Parse two user inputs with the shared voltage parser. Blank bounds mean unknown;
    /// two blanks remove the entry. Blank is distinct from an explicit zero.
    pub fn parse(terminal: &str, min: &str, max: &str) -> Result<Option<Self>, String> {
        let parse = |field, text: &str| {
            if text.trim().is_empty() {
                Ok(None)
            } else {
                Quantity::Voltage.parse(text).map(|v| Some(Value::from(v))).map_err(|e| format!("Terminal {terminal}: {field}: {e}"))
            }
        };
        let range = Self { supply_min: parse("supply_min", min)?, supply_max: parse("supply_max", max)? };
        if range.supply_min.is_none() && range.supply_max.is_none() {
            return Ok(None);
        }
        range.validate(terminal)?;
        Ok(Some(range))
    }
}

/// Borrow the exact numeric string after enforcing its type and base unit, naming the bound on failure.
fn voltage<'a>(terminal: &str, field: &str, bound: &'a Value) -> Result<&'a str, String> {
    let Value::Quantity { value, unit } = bound else {
        return Err(format!("Terminal {terminal}: {field}: expected a voltage quantity in V."));
    };
    if unit != Quantity::Voltage.unit() {
        return Err(format!("Terminal {terminal}: {field}: expected unit V, got {unit}."));
    }
    if !decimal(value) {
        return Err(format!("Terminal {terminal}: {field}: expected a canonical decimal in V."));
    }
    Ok(value)
}

/// Compare canonical decimals without scaling or rounding. Whole digits have no leading
/// zeroes; canonical fractional tails have no trailing zeroes, so prefix order is numeric.
fn compare_decimal(a: &str, b: &str) -> Ordering {
    let magnitude = |a: &str, b: &str| {
        let (a_whole, a_fraction) = a.split_once('.').unwrap_or((a, ""));
        let (b_whole, b_fraction) = b.split_once('.').unwrap_or((b, ""));
        a_whole.len().cmp(&b_whole.len()).then_with(|| a_whole.cmp(b_whole)).then_with(|| a_fraction.cmp(b_fraction))
    };
    match (a.strip_prefix('-'), b.strip_prefix('-')) {
        (Some(a), Some(b)) => magnitude(b, a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => magnitude(a, b),
    }
}
