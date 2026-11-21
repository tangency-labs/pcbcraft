//! Ball-grid pad identifier generation and natural identifier ordering.
//!
//! Row labels follow the standard bijective alphabetic sequence (`A..Z`, then `AA..`),
//! optionally omitting the six JEDEC skip letters (`I`, `O`, `Q`, `S`, `X`, `Z`) per
//! JEDEC JEP95 / IPC-SM-782A §14.1.3 practice. Natural ordering is for presentation,
//! browsing and inspection only; stored package maps retain their canonical key order.

use std::cmp::Ordering;

use crate::{board::definition::MAX_PADS, schematic::PinNumber};

/// Full 26-letter uppercase ASCII alphabet used when JEDEC letter skipping is disabled.
const FULL_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// 20-letter JEDEC ball-grid row alphabet omitting `I`, `O`, `Q`, `S`, `X` and `Z`.
const JEDEC_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPRTUVWY";

/// Letters omitted from ball-grid row identifiers when JEDEC skipping is enabled.
pub const JEDEC_SKIP_LETTERS: [char; 6] = ['I', 'O', 'Q', 'S', 'X', 'Z'];

/// Convert a zero-based row index into a ball-grid row label (`A..Z`, then `AA..`).
///
/// When `skip_jedec` is `true`, the 20-letter alphabet omitting `I`, `O`, `Q`, `S`, `X` and `Z`
/// is used in both single-letter and multi-letter labels (so row 20 is `Y`, row 21 is `AA`,
/// and row 26 is `AF`). When `false`, all 26 letters `A..Z` are used before wrapping to `AA`.
pub fn row_label(row: usize, skip_jedec: bool) -> String {
    let alphabet = if skip_jedec { JEDEC_ALPHABET } else { FULL_ALPHABET };
    let base = alphabet.len();
    let mut n = row.saturating_add(1);
    let mut letters = Vec::new();
    while n > 0 {
        n -= 1;
        letters.push(alphabet[n % base] as char);
        n /= base;
    }
    letters.reverse();
    letters.into_iter().collect()
}

/// Generate row-major ball-grid pad identifiers (`A1..`) for a `rows` by `columns` grid.
///
/// # Errors
/// Returns an error if either dimension is zero, if `rows * columns` exceeds [`MAX_PADS`],
/// or if any generated label fails [`PinNumber`] validation.
pub fn generate_identifiers(rows: u16, columns: u16, skip_jedec: bool) -> Result<Vec<String>, String> {
    if rows == 0 || columns == 0 {
        return Err("Ball-grid dimensions must both be at least 1.".into());
    }
    let total = usize::from(rows).checked_mul(usize::from(columns)).ok_or("Ball-grid pad count overflow.")?;
    if total > MAX_PADS {
        return Err(format!("Ball-grid array has {total} pads, exceeding the {MAX_PADS} pad limit."));
    }
    let mut names = Vec::with_capacity(total);
    for row in 0..usize::from(rows) {
        let prefix = row_label(row, skip_jedec);
        for column in 1..=usize::from(columns) {
            let name = format!("{prefix}{column}");
            if PinNumber::new(&name).is_none() {
                return Err(format!("Generated pad identifier `{name}` is not a valid pin number."));
            }
            names.push(name);
        }
    }
    Ok(names)
}

/// A maximal contiguous token used by [`natural_cmp`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Run<'a> {
    Digits(&'a str),
    Letters(&'a str),
    Other(&'a str),
}

impl Run<'_> {
    /// Assign a deterministic category rank so mismatched token kinds form a strict total order.
    fn rank(self) -> u8 {
        match self {
            Self::Digits(_) => 0,
            Self::Letters(_) => 1,
            Self::Other(_) => 2,
        }
    }
}

/// Extract the next maximal digit, letter, or punctuation run from `input`.
fn next_run(input: &str) -> Option<(Run<'_>, &str)> {
    let first = input.chars().next()?;
    if first.is_ascii_digit() {
        let end = input.find(|c: char| !c.is_ascii_digit()).unwrap_or(input.len());
        Some((Run::Digits(&input[..end]), &input[end..]))
    } else if first.is_ascii_alphabetic() {
        let end = input.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(input.len());
        Some((Run::Letters(&input[..end]), &input[end..]))
    } else {
        let end = input.find(|c: char| c.is_ascii_alphanumeric()).unwrap_or(input.len());
        Some((Run::Other(&input[..end]), &input[end..]))
    }
}

/// Compare two digit runs numerically without integer overflow, breaking leading-zero ties by spelling.
fn cmp_digits(a: &str, b: &str) -> Ordering {
    let a_trim = a.trim_start_matches('0');
    let b_trim = b.trim_start_matches('0');
    a_trim.len().cmp(&b_trim.len()).then_with(|| a_trim.cmp(b_trim)).then_with(|| a.len().cmp(&b.len())).then_with(|| a.cmp(b))
}

/// Compare two letter runs by length first (`Y` before `AA`), then alphabetically.
fn cmp_letters(a: &str, b: &str) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.bytes().map(|c| c.to_ascii_uppercase()).cmp(b.bytes().map(|c| c.to_ascii_uppercase())))
        .then_with(|| a.cmp(b))
}

/// Compare two pad or terminal identifiers in natural order for display and inspection.
///
/// Letter runs compare by length before alphabet (`Y` before `AA`, `Z` before `AA`), and
/// digit runs compare numerically (`A2` before `A10`, `2` before `10`). Leading-zero spellings
/// remain distinct (`01` and `1` never compare equal).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut rest_a = a;
    let mut rest_b = b;
    loop {
        match (next_run(rest_a), next_run(rest_b)) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((run_a, tail_a)), Some((run_b, tail_b))) => {
                let ord = match (run_a, run_b) {
                    (Run::Digits(x), Run::Digits(y)) => cmp_digits(x, y),
                    (Run::Letters(x), Run::Letters(y)) => cmp_letters(x, y),
                    (Run::Other(x), Run::Other(y)) => x.cmp(y),
                    _ => run_a.rank().cmp(&run_b.rank()),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
                rest_a = tail_a;
                rest_b = tail_b;
            }
        }
    }
}

/// Sort a slice of pad or terminal identifiers in place using [`natural_cmp`].
pub fn natural_sort<S: AsRef<str>>(items: &mut [S]) {
    items.sort_by(|a, b| natural_cmp(a.as_ref(), b.as_ref()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JEDEC skip mode omits I, O, Q, S, X and Z in both single-letter and multi-letter rows.
    #[test]
    fn jedec_skip_set_omits_forbidden_letters_and_wraps_to_aa() {
        let rows: Vec<String> = (0..42).map(|r| row_label(r, true)).collect();
        for label in &rows {
            for forbidden in JEDEC_SKIP_LETTERS {
                assert!(!label.contains(forbidden), "row `{label}` contains skipped letter `{forbidden}`");
            }
        }
        assert_eq!(rows[0], "A");
        assert_eq!(rows[7], "H");
        assert_eq!(rows[8], "J");
        assert_eq!(rows[12], "N");
        assert_eq!(rows[13], "P");
        assert_eq!(rows[14], "R");
        assert_eq!(rows[15], "T");
        assert_eq!(rows[18], "W");
        assert_eq!(rows[19], "Y");
        assert_eq!(rows[20], "AA");
        assert_eq!(rows[25], "AF");
        assert_eq!(rows[27], "AH");
        assert_eq!(rows[28], "AJ");
        assert_eq!(rows[39], "AY");
        assert_eq!(rows[40], "BA");
    }

    /// Without JEDEC skipping, rows run A..Z and then wrap to AA..AZ, BA...
    #[test]
    fn full_alphabet_wraps_past_z_to_aa() {
        assert_eq!(row_label(0, false), "A");
        assert_eq!(row_label(8, false), "I");
        assert_eq!(row_label(24, false), "Y");
        assert_eq!(row_label(25, false), "Z");
        assert_eq!(row_label(26, false), "AA");
        assert_eq!(row_label(27, false), "AB");
        assert_eq!(row_label(51, false), "AZ");
        assert_eq!(row_label(52, false), "BA");
    }

    /// Invalid or oversized ball grids are refused without panicking.
    #[test]
    fn bad_grid_dimensions_are_refused() {
        assert_eq!(generate_identifiers(0, 4, true).unwrap_err(), "Ball-grid dimensions must be at least 1x1.");
        assert_eq!(generate_identifiers(4, 0, true).unwrap_err(), "Ball-grid dimensions must be at least 1x1.");
        assert_eq!(generate_identifiers(0, 0, false).unwrap_err(), "Ball-grid dimensions must be at least 1x1.");
        assert!(generate_identifiers(33, 32, true).unwrap_err().contains("pad limit"));
        assert!(generate_identifiers(1, 1025, true).unwrap_err().contains("pad limit"));
        assert_eq!(generate_identifiers(32, 32, true).map(|v| v.len()), Ok(1024));
    }

    /// Natural ordering places A2 before A10, Y before AA, and forms a strict total order.
    #[test]
    fn natural_sort_orders_ball_grid_and_numeric_identifiers() {
        let mut ids = vec!["AA1", "A10", "Y26", "A2", "A1", "AF26", "Y2", "AA10", "B1", "10", "2", "1"];
        natural_sort(&mut ids);
        assert_eq!(ids, vec!["1", "2", "10", "A1", "A2", "A10", "B1", "Y2", "Y26", "AA1", "AA10", "AF26"]);
        assert_eq!(natural_cmp("A2", "A10"), Ordering::Less);
        assert_eq!(natural_cmp("Y", "AA"), Ordering::Less);
        assert_eq!(natural_cmp("Z", "AA"), Ordering::Less);
        assert_eq!(natural_cmp("Y26", "AA1"), Ordering::Less);
        assert_ne!(natural_cmp("01", "1"), Ordering::Equal);
        assert_eq!(natural_cmp("01", "1"), natural_cmp("1", "01").reverse());
    }
}
