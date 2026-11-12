//! Physical quantities, units and geometry length input, independent of UI or project state.
//!
//! [`quantity`] supplies canonical physical values and the standard unit registry. The geometry
//! length API below retains its existing integer-nanometre conversion and editable text contract.
//!
//! Geometry lengths in command payloads remain whole integer nanometres. A caller supplies the default
//! unit for bare text; a suffix overrides it. Conversion rounds once to the nearest nanometre,
//! while callers apply their own coordinate, thickness or clearance constraints afterward.
//!
//! ```
//! use pcbcraft_units::{Unit, edit_label, parse_length};
//!
//! assert_eq!(parse_length("100 mil", Unit::Mm), Ok(2_540_000));
//! assert_eq!(parse_length("2.54 mm", Unit::Mil), Ok(2_540_000));
//! let draft = edit_label(-25_401, Unit::Mil);
//! assert_eq!(parse_length(&draft, Unit::Mm), Ok(-25_401));
//! ```

/// Canonical physical quantities and standard unit definitions shared by every host.
pub mod quantity;

use std::fmt;

use serde::{Deserialize, Serialize};

/// Nanometres in one mil (a thousandth of an inch), exactly.
pub const NM_PER_MIL: i64 = 25_400;
/// Nanometres in one millimetre.
pub const NM_PER_MM: i64 = 1_000_000;
/// Nanometres in one inch.
const NM_PER_INCH: i64 = 1000 * NM_PER_MIL;
/// The largest length [`parse_length`] accepts either side of zero: one kilometre, well inside
/// the range where every nanometre is exact in `f64`.
pub const MAX_LENGTH_NM: i64 = 1_000_000_000_000;

/// The explicit default for bare human-entered lengths and the unit of exact edit text.
/// Callers choose it; parsing never reads a view preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    /// Thousandths of an inch, the schematic grid's native unit.
    #[default]
    Mil,
    /// Millimetres, with one million whole nanometres per unit.
    Mm,
}

impl Unit {
    /// The unit's short name, as shown after a value: `mil` or `mm`.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Mil => "mil",
            Self::Mm => "mm",
        }
    }

    /// The other unit, for a control that switches between the two.
    pub fn toggled(self) -> Self {
        match self {
            Self::Mil => Self::Mm,
            Self::Mm => Self::Mil,
        }
    }

    /// Exact nanometres per unit; this factor does not apply field-specific bounds.
    pub const fn nm_per_unit(self) -> i64 {
        match self {
            Self::Mil => NM_PER_MIL,
            Self::Mm => NM_PER_MM,
        }
    }
}

/// Why typed text is not a length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LengthError {
    /// No non-whitespace input was supplied.
    Empty,
    /// The numeric portion is missing or malformed.
    NotANumber,
    /// The text after the number, which is not a known unit.
    Unit(String),
    /// Further from zero than [`MAX_LENGTH_NM`].
    OutOfRange,
}

impl fmt::Display for LengthError {
    /// Preserve the shared conversion diagnostics without applying field-specific validation.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Enter a length, such as 20 mil or 0.5 mm"),
            Self::NotANumber => write!(f, "Enter a number, such as 20 mil or 0.5 mm"),
            Self::Unit(unit) => write!(f, "Unknown unit \"{unit}\". Use mil, mm or in"),
            Self::OutOfRange => write!(f, "That length is too large"),
        }
    }
}

/// Read a typed length in whole nanometres. An optional `-` or `+` may lead; then a decimal
/// number; then, with or without a space and in any case, `mil`, `mils`, `th`, `thou`, `mm`, `in`
/// or `"`. A number with no unit is in `default`. The value is rounded once to the nearest
/// nanometre and must be within [`MAX_LENGTH_NM`] of zero. Exponents and unknown units are refused,
/// so nothing is guessed. Callers check any narrower range of their own.
pub fn parse_length(text: &str, default: Unit) -> Result<i64, LengthError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(LengthError::Empty);
    }
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let split = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
    let (number, unit) = rest.split_at(split);
    let nm_per_unit = match unit.trim().to_ascii_lowercase().as_str() {
        "" => default.nm_per_unit(),
        "mil" | "mils" | "th" | "thou" => NM_PER_MIL,
        "mm" => NM_PER_MM,
        "in" | "\"" => NM_PER_INCH,
        _ if number.is_empty() => return Err(LengthError::NotANumber),
        other => return Err(LengthError::Unit(other.to_owned())),
    };
    if !number.bytes().any(|b| b.is_ascii_digit()) {
        return Err(LengthError::NotANumber);
    }
    let value: f64 = number.parse().map_err(|_| LengthError::NotANumber)?;
    let nm = (value * nm_per_unit as f64).round();
    if !nm.is_finite() || nm > MAX_LENGTH_NM as f64 {
        return Err(LengthError::OutOfRange);
    }
    let nm = nm as i64;
    Ok(if negative { -nm } else { nm })
}

/// Format an editable length with enough precision to round-trip whole nanometres through
/// [`parse_length`] within [`MAX_LENGTH_NM`]. An explicit suffix keeps a draft's physical meaning
/// when the display unit changes. Display-only rounding must remain separate from editable text.
pub fn edit_label(nm: i64, unit: Unit) -> String {
    let decimals = match unit {
        Unit::Mil => 5,
        Unit::Mm => 6,
    };
    let text = format!("{:.*}", decimals, nm as f64 / unit.nm_per_unit() as f64);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} {}", unit.symbol())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact relations the issue names hold both ways, and a value typed in either unit
    /// lands on the same nanometre.
    #[test]
    fn mils_and_millimetres_convert_exactly() {
        assert_eq!(parse_length("100 mil", Unit::Mm), Ok(2_540_000));
        assert_eq!(parse_length("2.54 mm", Unit::Mil), Ok(2_540_000));
        assert_eq!(parse_length("1 mil", Unit::Mm), parse_length("0.0254", Unit::Mm));
        assert_eq!(parse_length("1 in", Unit::Mm), Ok(1000 * NM_PER_MIL));
        assert_eq!(parse_length("0.5\"", Unit::Mm), Ok(500 * NM_PER_MIL));
    }

    /// Editing an unchanged field must preserve every nanometre, including values that labels
    /// round to zero and coordinates near the accepted extremes, in either display unit.
    #[test]
    fn editable_lengths_round_trip_nanometres() {
        for unit in [Unit::Mil, Unit::Mm] {
            for nm in [0, 1, -1, 25_401, -123_456_789, 987_299_999, MAX_LENGTH_NM - 1, -MAX_LENGTH_NM] {
                assert_eq!(parse_length(&edit_label(nm, unit), unit.toggled()), Ok(nm), "{nm} {unit:?}");
            }
        }
    }

    /// A bare number takes the explicit default; a suffix overrides it in any case and spacing.
    #[test]
    fn bare_numbers_use_the_explicit_default_and_suffixes_override_it() {
        assert_eq!(parse_length("50", Unit::Mil), Ok(1_270_000));
        assert_eq!(parse_length("50", Unit::Mm), Ok(50 * NM_PER_MM));
        assert_eq!(parse_length("1.27MM", Unit::Mil), Ok(1_270_000));
        assert_eq!(parse_length(" 50 Mils ", Unit::Mm), Ok(1_270_000));
        assert_eq!(parse_length("50th", Unit::Mm), Ok(1_270_000));
        assert_eq!(parse_length(".5 mm", Unit::Mil), Ok(500_000));
        assert_eq!(parse_length("5.", Unit::Mm), Ok(5 * NM_PER_MM));
    }

    /// Both signs parse, and negative zero becomes the canonical integer zero.
    #[test]
    fn signed_lengths_parse_to_canonical_nanometres() {
        assert_eq!(parse_length("-25 mil", Unit::Mm), Ok(-635_000));
        assert_eq!(parse_length("+1.5", Unit::Mm), Ok(1_500_000));
        assert_eq!(parse_length("-0", Unit::Mil), Ok(0));
    }

    /// Rounding happens once, at the nanometre, even far from zero; the limit is a kilometre.
    #[test]
    fn precision_holds_near_the_limits() {
        assert_eq!(parse_length("0.0000004 mm", Unit::Mil), Ok(0));
        assert_eq!(parse_length("0.0000005 mm", Unit::Mil), Ok(1));
        assert_eq!(parse_length("1000000 mm", Unit::Mil), Ok(MAX_LENGTH_NM));
        assert_eq!(parse_length("-1000000 mm", Unit::Mil), Ok(-MAX_LENGTH_NM));
        assert_eq!(parse_length("1000000.000001 mm", Unit::Mil), Err(LengthError::OutOfRange));
        assert_eq!(parse_length("123456.789012 mm", Unit::Mil), Ok(123_456_789_012));
        assert_eq!(parse_length(&"9".repeat(400), Unit::Mm), Err(LengthError::OutOfRange));
    }

    /// Text that isn't a plain length is refused with the reason, never guessed at.
    #[test]
    fn invalid_text_is_refused() {
        assert_eq!(parse_length("  ", Unit::Mil), Err(LengthError::Empty));
        assert_eq!(parse_length("mil", Unit::Mil), Err(LengthError::NotANumber));
        assert_eq!(parse_length("-", Unit::Mil), Err(LengthError::NotANumber));
        assert_eq!(parse_length(".", Unit::Mil), Err(LengthError::NotANumber));
        assert_eq!(parse_length("1.2.3", Unit::Mil), Err(LengthError::NotANumber));
        assert_eq!(parse_length("--5", Unit::Mil), Err(LengthError::NotANumber));
        assert_eq!(parse_length("1e3", Unit::Mil), Err(LengthError::Unit("e3".into())));
        assert_eq!(parse_length("5 cm", Unit::Mil), Err(LengthError::Unit("cm".into())));
        assert_eq!(parse_length("5 mm mil", Unit::Mil), Err(LengthError::Unit("mm mil".into())));
        assert_eq!(LengthError::Unit("cm".into()).to_string(), "Unknown unit \"cm\". Use mil, mm or in");
    }

    /// Unit identity, default and toggling stay independent of any view.
    #[test]
    fn the_unit_toggles_between_mil_and_mm() {
        assert_eq!(Unit::default(), Unit::Mil);
        assert_eq!(Unit::Mil.toggled(), Unit::Mm);
        assert_eq!(Unit::Mm.toggled().symbol(), "mil");
    }
}
