//! Aggregate accounting for one library check. Charges are made on borrowed inputs before
//! copying them or adding entries to temporary or retained collections.
use super::{CaptureSource, CheckError, Reference};

/// Work units for captured structure, string bytes and repeated evaluation passes.
pub(super) const MAX_WORK: usize = 32 * 1024 * 1024;
/// Upper bound on compact report bytes, including repeated subjects and occurrence lists.
pub(super) const MAX_REPORT_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct Budget {
    pub(super) remaining: usize,
    limit: usize,
    what: &'static str,
}

impl Budget {
    pub(super) fn work(limit: usize) -> Self {
        Self { remaining: limit, limit, what: "Library check work units" }
    }

    pub(super) fn report(limit: usize) -> Self {
        Self { remaining: limit, limit, what: "Library check report byte budget" }
    }

    pub(super) fn charge(&mut self, units: usize) -> Result<(), CheckError> {
        self.remaining = self.remaining.checked_sub(units).ok_or_else(|| CheckError::TooLarge { what: self.what.into(), limit: self.limit })?;
        Ok(())
    }

    /// Per-entry overhead, plus a byte-weighted charge even for unchecked input strings.
    pub(super) fn text(&mut self, text: &str) -> Result<(), CheckError> {
        self.charge(32usize.saturating_add(text.len().saturating_mul(6)))
    }

    pub(super) fn reference(&mut self, reference: &Reference) -> Result<(), CheckError> {
        self.text(&reference.library)?;
        self.text(&reference.id)
    }

    pub(super) fn source(&mut self, source: &CaptureSource) -> Result<(), CheckError> {
        if let CaptureSource::Document { path } = source {
            self.text(path)?;
        }
        Ok(())
    }

    /// Account for the report envelope even when there are no findings.
    pub(super) fn envelope(&mut self) -> Result<(), CheckError> {
        self.charge(256)
    }

    /// Per finding, 512 covers fixed JSON fields, static strings, numeric fields and separators.
    /// A pad occurrence needs at most 48 bytes including its comma (u64 index and u16 ordinal).
    /// The only variable text is in the subject; JSON escaping takes at most 6 bytes per UTF-8
    /// byte. Charge each copy, even when all findings refer to the same part or document.
    pub(super) fn finding(
        &mut self,
        reference: &Reference,
        source: &CaptureSource,
        package: Option<&str>,
        number: &str,
        occurrences: usize,
    ) -> Result<(), CheckError> {
        self.charge(512usize.saturating_add(48usize.saturating_mul(occurrences)))?;
        self.reference(reference)?;
        self.source(source)?;
        if let Some(package) = package {
            self.text(package)?;
        }
        self.text(number)
    }
}
