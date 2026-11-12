//! Canonical base units and exact decimal input normalization.
use super::Value;
use serde::{Deserialize, Serialize};
/// Physical quantities use one canonical unit each; display-unit parsing is a separate operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantity {
    Resistance,
    Capacitance,
    Inductance,
    Voltage,
    Current,
    Power,
    Frequency,
    Time,
    Length,
    Temperature,
    Ratio,
}
impl Quantity {
    /// Canonical case-sensitive unit identifiers avoid interpreting display labels or SI prefixes.
    pub fn unit(self) -> &'static str {
        match self {
            Self::Resistance => "Ohm",
            Self::Capacitance => "F",
            Self::Inductance => "H",
            Self::Voltage => "V",
            Self::Current => "A",
            Self::Power => "W",
            Self::Frequency => "Hz",
            Self::Time => "s",
            Self::Length => "m",
            Self::Temperature => "K",
            Self::Ratio => "1",
        }
    }

    /// Normalize unit-aware input into an exact base-unit value. Prefixes are case-sensitive;
    /// omitted units use this quantity, and incompatible units or excess precision are rejected.
    pub fn parse(self, text: &str) -> Result<Value, String> {
        if text.len() > 128 {
            return Err("Quantity input exceeds 128 bytes.".into());
        }
        let text = text.trim();
        let number_end = text.find(|c: char| !c.is_ascii_digit() && !matches!(c, '+' | '-' | '.' | 'e' | 'E')).unwrap_or(text.len());
        let (number, suffix) = text.split_at(number_end);
        let suffix: String = suffix.chars().filter(|c| !c.is_whitespace()).collect();
        let aliases: &[&str] = match self {
            Self::Resistance => &["Ohm", "ohm", "ohms", "Ω", "R"],
            _ => &[self.unit()],
        };
        let shift = if suffix.is_empty() || aliases.contains(&suffix.as_str()) {
            0
        } else if self == Self::Ratio && suffix == "%" {
            -2
        } else {
            let prefixes =
                [("p", -12), ("n", -9), ("u", -6), ("µ", -6), ("μ", -6), ("m", -3), ("k", 3), ("M", 6), ("G", 9), ("T", 12), ("milli", -3)];
            prefixes
                .into_iter()
                .find_map(|(prefix, shift)| suffix.strip_prefix(prefix).filter(|rest| rest.is_empty() || aliases.contains(rest)).map(|_| shift))
                .ok_or_else(|| format!("Expected {} or a compatible SI-prefixed unit.", self.unit()))?
        };
        let value = normalized_decimal(number, shift)?;
        Ok(Value::Quantity { value, unit: self.unit().into() })
    }
}

/// Canonical decimals have at most 36 digits and 18 fractional places; zero has exactly one spelling.
pub fn decimal(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    let (whole, fraction) = digits.split_once('.').map_or((digits, None), |(a, b)| (a, Some(b)));
    !whole.is_empty()
        && (whole == "0" || !whole.starts_with('0'))
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.is_none_or(|f| !f.is_empty() && f.len() <= 18 && !f.ends_with('0') && f.bytes().all(|b| b.is_ascii_digit()))
        && whole.len() + fraction.map_or(0, str::len) <= 36
        && value != "-0"
}

/// Shift decimal text by a power of ten without binary floating-point conversion or rounding.
fn normalized_decimal(number: &str, shift: i32) -> Result<String, String> {
    let (mantissa, exponent) = match number.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().map_err(|_| "Invalid decimal exponent.")?),
        None => (number, 0),
    };
    if !(-48..=48).contains(&exponent) {
        return Err("Decimal exponent is outside supported bounds.".into());
    }
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.strip_prefix(['-', '+']).unwrap_or(mantissa);
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (whole.is_empty() && fraction.is_empty()) || !whole.bytes().chain(fraction.bytes()).all(|b| b.is_ascii_digit()) {
        return Err("Invalid decimal quantity.".into());
    }
    let digits = format!("{whole}{fraction}");
    let mut digits = digits.trim_start_matches('0').to_owned();
    if digits.is_empty() {
        return Ok("0".into());
    }
    let mut scale = exponent + shift - i32::try_from(fraction.len()).map_err(|_| "Quantity precision overflow.")?;
    while digits.ends_with('0') {
        digits.pop();
        scale += 1;
    }
    let value = if scale >= 0 {
        format!("{digits}{}", "0".repeat(scale as usize))
    } else {
        let split = digits.len() as i32 + scale;
        if split > 0 {
            let (a, b) = digits.split_at(split as usize);
            format!("{a}.{b}")
        } else {
            format!("0.{}{digits}", "0".repeat((-split) as usize))
        }
    };
    let value = if negative { format!("-{value}") } else { value };
    if decimal(&value) { Ok(value) } else { Err("Quantity exceeds 36 decimal digits or 18 fractional places; no rounding was applied.".into()) }
}
