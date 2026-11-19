//! Exact JSON integer input for the netlist's u32 revisions, independent of number spelling.

use serde::Deserialize;

/// Read a JSON number whose mathematical value is a u32. Decimal and exponent spellings
/// are accepted without rounding; serialization still uses the field's ordinary u32 writer.
pub(super) fn revision<'de, D: serde::Deserializer<'de>>(de: D) -> Result<u32, D::Error> {
    let raw = Box::<serde_json::value::RawValue>::deserialize(de)?;
    exact_u32(raw.get()).ok_or_else(|| serde::de::Error::custom("revision must be an integer from 0 to 4294967295"))
}

/// Convert a validated JSON number token to u32, or refuse a non-number, fraction or overflow.
/// Zero is independent of its exponent and sign. For nonzero values, cancelling decimal
/// zeros before checking the scale prevents both float rounding and large powers of ten.
fn exact_u32(text: &str) -> Option<u32> {
    let (negative, unsigned) = text.strip_prefix('-').map_or((false, text), |text| (true, text));
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty() || !whole.bytes().chain(fraction.bytes()).all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let digits = mantissa.replace('.', "");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some(0);
    }
    if negative {
        return None;
    }
    let coefficient = significant.trim_end_matches('0');
    let trailing = i64::try_from(significant.len() - coefficient.len()).ok()?;
    let scale = exponent.parse::<i64>().ok()?.checked_sub(i64::try_from(fraction.len()).ok()?)?.checked_add(trailing)?;
    if !(0..=9).contains(&scale) || coefficient.len().checked_add(usize::try_from(scale).ok()?)? > 10 {
        return None;
    }
    coefficient.parse::<u32>().ok()?.checked_mul(10_u32.checked_pow(u32::try_from(scale).ok()?)?)
}
