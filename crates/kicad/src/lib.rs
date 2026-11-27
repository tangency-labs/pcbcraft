//! Read KiCad files into KiCad-shaped models. Nothing here knows PCBCraft's own model: the
//! engine maps what is read, and decides what it can keep.
//!
//! The s-expression reader and much of the schematic reading are adapted from CircuitCraft
//! (MIT OR Apache-2.0; see ATTRIBUTION.md), whose readers were written from KiCad's published
//! file format documentation (<https://dev-docs.kicad.org/en/file-formats/>) and from reading
//! schematic files of public designs saved by KiCad. No KiCad source, headers, tests or
//! source-derived fixtures were consulted. This crate's test fixtures are written by hand.
//!
//! Every file is treated as hostile: sizes, nesting and counts are bounded, nothing panics, and
//! what a reader can't use is counted in its [`Notes`] instead of being guessed at.

pub mod schematic;
mod sexp;

pub use schematic::{Schematic, read_schematic};

use std::collections::BTreeMap;
use std::fmt;

/// Why a file could not be read at all. A file that can be read in part is read, with notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The text is not well-formed s-expressions; `line` counts from 1.
    Syntax { line: usize, message: String },
    /// The text is well-formed but is another kind of file; names the kind expected.
    WrongFormat(&'static str),
    /// The file is larger than a reader accepts.
    TooLarge(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax { line, message } => write!(f, "line {line}: {message}"),
            Error::WrongFormat(expected) => write!(f, "this is not {expected}"),
            Error::TooLarge(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for Error {}

/// What a reader left out, by description, with how often each happened. Ordered by
/// description, so the same file always gives the same notes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notes(BTreeMap<String, usize>);

impl Notes {
    /// Count one more of `what`.
    pub(crate) fn add(&mut self, what: &str) {
        let count = self.0.entry(what.to_owned()).or_default();
        *count = count.saturating_add(1);
    }

    /// Each description with its count.
    pub fn iter(&self) -> impl Iterator<Item = (&str, usize)> {
        self.0.iter().map(|(what, count)| (what.as_str(), *count))
    }

    /// Whether nothing was left out.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A point in nanometres. Each field that holds one says which frame it is in: sheet points
/// have y growing downward, as the file writes them; symbol-library points have y growing
/// upward, as the format defines them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub x_nm: i64,
    pub y_nm: i64,
}

/// The largest coordinate or length a reader accepts, in nanometres: one kilometre. KiCad's own
/// sheets are far smaller; the bound keeps arithmetic on what is read from overflowing.
pub const MAX_NM: i64 = 1_000_000_000_000;

/// Millimetres from a file as whole nanometres, rounded; `None` beyond [`MAX_NM`].
pub(crate) fn nm(mm: f64) -> Option<i64> {
    let value = (mm * 1_000_000.0).round();
    // The bound is exactly representable, so the comparison is exact and the cast can't saturate.
    (value.abs() <= MAX_NM as f64).then_some(value as i64)
}

/// Degrees from a file as millidegrees in `0..360_000`, rounded.
pub(crate) fn millidegrees(degrees: f64) -> i32 {
    let value = (degrees.rem_euclid(360.0) * 1000.0).round() as i64;
    // Rounding can reach 360 000 exactly; that is a whole turn, which is no turn.
    (value % 360_000) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millimetres_round_to_nanometres_within_bounds() {
        assert_eq!(nm(2.54), Some(2_540_000));
        assert_eq!(nm(-0.0000004), Some(0));
        assert_eq!(nm(0.0000006), Some(1));
        assert_eq!(nm(1_000_000.0), Some(MAX_NM));
        assert_eq!(nm(1_000_000.001), None);
        assert_eq!(nm(-1e300), None);
    }

    #[test]
    fn degrees_normalize_to_one_turn() {
        assert_eq!(millidegrees(90.0), 90_000);
        assert_eq!(millidegrees(-90.0), 270_000);
        assert_eq!(millidegrees(720.0), 0);
        assert_eq!(millidegrees(359.9999), 0);
        assert_eq!(millidegrees(45.5), 45_500);
    }

    #[test]
    fn notes_count_by_description_in_order() {
        let mut notes = Notes::default();
        assert!(notes.is_empty());
        notes.add("text box left out");
        notes.add("bus left out");
        notes.add("text box left out");
        assert_eq!(notes.iter().collect::<Vec<_>>(), [("bus left out", 1), ("text box left out", 2)]);
    }
}
