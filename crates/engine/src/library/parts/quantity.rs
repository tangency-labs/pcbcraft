//! Canonical base units and exact decimal input normalization.
use super::Value;
use serde::{Deserialize, Serialize};

/// Ordered engineering prefixes shared by input and display; the empty entry is the base unit.
const ENGINEERING_PREFIXES: [(&str, i32); 13] =
    [("a", -18), ("f", -15), ("p", -12), ("n", -9), ("µ", -6), ("m", -3), ("", 0), ("k", 3), ("M", 6), ("G", 9), ("T", 12), ("P", 15), ("E", 18)];
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
        let (number, suffix) = text.split_at(number_end(text));
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
            ENGINEERING_PREFIXES
                .iter()
                .copied()
                .filter(|(prefix, _)| !prefix.is_empty())
                .chain([("u", -6), ("μ", -6), ("milli", -3)])
                .find_map(|(prefix, shift)| suffix.strip_prefix(prefix).filter(|rest| rest.is_empty() || aliases.contains(rest)).map(|_| shift))
                .ok_or_else(|| format!("Expected {} or a compatible SI-prefixed unit.", self.unit()))?
        };
        let value = normalized_decimal(number, shift)?;
        Ok(Value::Quantity { value, unit: self.unit().into() })
    }
}

/// Consume an optional scientific exponent only when it has digits, leaving a bare
/// uppercase E available as the exa prefix. Malformed numbers still fail normalization.
fn number_end(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut end = bytes.iter().position(|byte| !byte.is_ascii_digit() && !matches!(byte, b'+' | b'-' | b'.')).unwrap_or(bytes.len());
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let start = end + 1;
        let digits = start + usize::from(matches!(bytes.get(start), Some(b'+' | b'-')));
        if bytes.get(digits).is_some_and(u8::is_ascii_digit) {
            end = digits + bytes[digits..].iter().take_while(|byte| byte.is_ascii_digit()).count();
        }
    }
    end
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

/// Readable text for a canonical base-unit quantity, such as `4700`/`Ohm` as `4.7 kΩ`.
///
/// Precision: every significant digit is kept and nothing is rounded, so [`Quantity::parse`]
/// returns the same canonical value for the text. Prefix: the engineering prefix from a to E that
/// puts one to three digits before the decimal point, clamped at a and E; zero has no prefix.
/// Ratios (`1`) show the plain decimal; non-canonical input is shown unscaled with its unit.
pub fn display_quantity(value: &str, unit: &str) -> String {
    if unit == "1" {
        return value.to_owned();
    }
    let symbol = if unit == "Ohm" { "Ω" } else { unit };
    if !decimal(value) || value == "0" {
        return format!("{value} {symbol}");
    }
    let digits = value.trim_start_matches('-');
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let magnitude = if whole == "0" { -1 - (fraction.len() - fraction.trim_start_matches('0').len()) as i32 } else { whole.len() as i32 - 1 };
    let (prefix, exponent) =
        ENGINEERING_PREFIXES.iter().rev().copied().find(|(_, exponent)| magnitude >= *exponent).unwrap_or(ENGINEERING_PREFIXES[0]);
    let scaled = shift_decimal(value, -exponent).unwrap_or_else(|_| value.to_owned());
    format!("{scaled} {prefix}{symbol}")
}

/// Shift decimal text by a power of ten and reject results that are not canonical stored decimals.
fn normalized_decimal(number: &str, shift: i32) -> Result<String, String> {
    let value = shift_decimal(number, shift)?;
    if decimal(&value) { Ok(value) } else { Err("Quantity exceeds 36 decimal digits or 18 fractional places; no rounding was applied.".into()) }
}

/// Shift decimal text by a power of ten without binary floating-point conversion or rounding.
/// Errors on malformed digits or an exponent outside ±48; the result has no trailing zeros.
fn shift_decimal(number: &str, shift: i32) -> Result<String, String> {
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
    Ok(if negative { format!("-{value}") } else { value })
}
