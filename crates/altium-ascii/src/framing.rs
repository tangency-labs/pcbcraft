//! Record and property framing for Altium ASCII saves.
//!
//! A file is a sequence of records. Each record is a list of `NAME=VALUE` properties separated
//! by `|`, normally one record per line and ending with a line break (CR LF or LF). Framing does
//! not assume one record per line:
//!
//! - A text value may hold raw line breaks. When the last property on a line is one of the
//!   [`Dialect`]'s multi-line properties and the next line does not start a record, that line
//!   continues the value: the line break is kept in the value, and any `|NAME=VALUE` that
//!   follows on the continuation line belongs to the same record.
//! - A record may start a line with or without a leading `|`.
//! - Several records may share one line: a second record-kind property within one record starts
//!   the next record at that property.
//!
//! A line starts a new record when its first property (after an optional `|`) is the record-kind
//! property, or when the record before it does not end in an open multi-line value. Blank lines
//! between records are skipped. Names are compared without regard to ASCII case.
//!
//! Every record must end with a line break. A file that ends inside a record, a property without
//! `=`, an empty or over-long property name, and a record over the size or property limits are
//! each reported as a [`FrameError`] with the byte offset and line where it occurs. Iteration
//! stops after the first error, so nothing past a damaged record is read or guessed.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufRead, Read};

/// Limits applied while framing. Each one is checked before memory for the data it bounds is
/// allocated, so a damaged or hostile file cannot make the reader grow without bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest record, in bytes of the file from its first byte to the end of its last
    /// line, including the line breaks inside it.
    pub max_record_bytes: usize,
    /// The most properties one record may hold.
    pub max_properties: usize,
    /// The longest property name, in bytes.
    pub max_name_bytes: usize,
}

impl Default for Limits {
    /// Limits far above any record a board or schematic save is expected to hold, yet small
    /// enough that the reader's memory stays bounded: 16 MiB per record, 65,536 properties per
    /// record and 256-byte property names.
    fn default() -> Self {
        Self { max_record_bytes: 16 * 1024 * 1024, max_properties: 65_536, max_name_bytes: 256 }
    }
}

/// How a file marks the start of a record and which values may span lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dialect {
    kind_property: Option<Box<[u8]>>,
    multi_line_properties: Vec<Box<[u8]>>,
}

impl Dialect {
    /// A dialect with no record-kind property and no multi-line values: every line is a record.
    pub fn new() -> Self {
        Self::default()
    }

    /// Names the property that holds a record's kind. A line whose first property has this
    /// name always starts a record, and a second one inside a record starts the next record.
    pub fn with_kind_property(mut self, name: &str) -> Self {
        self.kind_property = Some(name.as_bytes().into());
        self
    }

    /// Adds a property whose value may hold raw line breaks.
    pub fn with_multi_line_property(mut self, name: &str) -> Self {
        self.multi_line_properties.push(name.as_bytes().into());
        self
    }

    /// The record-kind property's name, when the dialect has one.
    pub fn kind_property(&self) -> Option<&[u8]> {
        self.kind_property.as_deref()
    }

    /// Whether `name` is the record-kind property, ignoring ASCII case.
    pub fn is_kind_property(&self, name: &[u8]) -> bool {
        self.kind_property.as_deref().is_some_and(|kind| kind.eq_ignore_ascii_case(name))
    }

    /// Whether `name` is a multi-line property, ignoring ASCII case.
    pub fn is_multi_line_property(&self, name: &[u8]) -> bool {
        self.multi_line_properties.iter().any(|multi| multi.eq_ignore_ascii_case(name))
    }

    /// Whether a line (without its line break) starts with the record-kind property, with or
    /// without a leading `|`.
    fn starts_with_kind(&self, line: &[u8]) -> bool {
        let Some(kind) = self.kind_property.as_deref() else {
            return false;
        };
        let rest = line.strip_prefix(b"|").unwrap_or(line);
        rest.len() > kind.len() && rest[..kind.len()].eq_ignore_ascii_case(kind) && rest[kind.len()] == b'='
    }
}

/// One `NAME=VALUE` property of a record, with the raw bytes of both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    name: Vec<u8>,
    value: Vec<u8>,
    offset: u64,
    line: u64,
}

impl Property {
    /// The property's name, as written.
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// The property's value, as written; a multi-line value keeps its raw line breaks.
    pub fn value(&self) -> &[u8] {
        &self.value
    }

    /// The byte offset of the property's name in the file.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// The 1-based line on which the property's name stands.
    pub fn line(&self) -> u64 {
        self.line
    }
}

/// One record: its properties in file order and where it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    offset: u64,
    line: u64,
    properties: Vec<Property>,
    kind: Option<usize>,
}

impl Record {
    /// The byte offset in the file at which the record starts.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// The 1-based line on which the record starts.
    pub fn line(&self) -> u64 {
        self.line
    }

    /// The record's properties, in file order.
    pub fn properties(&self) -> &[Property] {
        &self.properties
    }

    /// The value of the record-kind property, when the record has one.
    pub fn kind(&self) -> Option<&[u8]> {
        self.kind.and_then(|index| self.properties.get(index)).map(Property::value)
    }

    /// The first property named `name`, ignoring ASCII case.
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|property| property.name.eq_ignore_ascii_case(name.as_bytes()))
    }

    /// The value of the first property named `name`, ignoring ASCII case.
    pub fn value(&self, name: &str) -> Option<&[u8]> {
        self.property(name).map(Property::value)
    }
}

/// Why framing stopped. Every variant names the byte offset and line where the problem is.
#[derive(Debug)]
pub enum FrameError {
    /// Reading the input failed.
    Io(io::Error),
    /// The file ends inside a record: its last line has no line break.
    Truncated {
        /// Where the cut record starts.
        record_offset: u64,
        /// The line on which the cut record starts.
        record_line: u64,
        /// The length of the file, where the record was cut.
        end_offset: u64,
    },
    /// A property has no `=` between its name and value.
    PropertyWithoutValue {
        /// Where the property starts.
        offset: u64,
        /// The line of the property.
        line: u64,
    },
    /// A property's name is empty.
    EmptyPropertyName {
        /// Where the property starts.
        offset: u64,
        /// The line of the property.
        line: u64,
    },
    /// A property's name is longer than [`Limits::max_name_bytes`].
    PropertyNameTooLong {
        /// Where the property starts.
        offset: u64,
        /// The line of the property.
        line: u64,
        /// The limit that was exceeded.
        limit: usize,
    },
    /// A record is larger than [`Limits::max_record_bytes`].
    RecordTooLarge {
        /// Where the record starts.
        record_offset: u64,
        /// The line on which the record starts.
        record_line: u64,
        /// The limit that was exceeded.
        limit: usize,
    },
    /// A record has more properties than [`Limits::max_properties`].
    TooManyProperties {
        /// Where the record starts.
        record_offset: u64,
        /// The line on which the record starts.
        record_line: u64,
        /// The limit that was exceeded.
        limit: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "reading the file failed: {error}"),
            Self::Truncated { record_offset, record_line, end_offset } => write!(
                f,
                "the file ends at byte {end_offset} inside the record that starts at byte \
                 {record_offset} (line {record_line})"
            ),
            Self::PropertyWithoutValue { offset, line } => write!(f, "the property at byte {offset} (line {line}) has no '=' before its value"),
            Self::EmptyPropertyName { offset, line } => {
                write!(f, "the property at byte {offset} (line {line}) has no name")
            }
            Self::PropertyNameTooLong { offset, line, limit } => {
                write!(f, "the property name at byte {offset} (line {line}) is longer than {limit} bytes")
            }
            Self::RecordTooLarge { record_offset, record_line, limit } => write!(
                f,
                "the record at byte {record_offset} (line {record_line}) is larger than \
                 {limit} bytes"
            ),
            Self::TooManyProperties { record_offset, record_line, limit } => write!(
                f,
                "the record at byte {record_offset} (line {record_line}) has more than \
                 {limit} properties"
            ),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// One line of the file, without its line break.
struct Line {
    /// The byte offset of the line's first byte.
    offset: u64,
    /// The line's 1-based number.
    number: u64,
    /// The line's length without its line break.
    len: usize,
    /// The line break that ended it: CR LF, LF, or empty at the end of the file.
    line_break: &'static [u8],
}

/// A record being assembled from one or more lines.
struct Builder {
    record: Record,
    /// File bytes taken by the record so far, including the line breaks inside it.
    bytes: usize,
    /// Whether the record's last line ended inside a multi-line value.
    open_multi_line: bool,
    /// The line break that ended the record's last line, kept for a continuation.
    pending_break: &'static [u8],
}

impl Builder {
    fn new(offset: u64, line: u64) -> Self {
        Self { record: Record { offset, line, properties: Vec::new(), kind: None }, bytes: 0, open_multi_line: false, pending_break: b"" }
    }
}

/// An iterator over the records of an ASCII save, read from any buffered input.
///
/// It yields each record as soon as the line after it shows the record is complete, keeps at
/// most one record and one line in memory, and stops after the first [`FrameError`].
pub struct Records<R> {
    input: R,
    dialect: Dialect,
    limits: Limits,
    /// The byte offset of the next unread byte.
    offset: u64,
    /// The number of the next line.
    line: u64,
    current: Option<Builder>,
    line_buf: Vec<u8>,
    ready: VecDeque<Record>,
    error: Option<FrameError>,
    finished: bool,
}

impl<R: BufRead> Records<R> {
    /// Starts framing `input` with the given dialect and limits.
    pub fn new(input: R, dialect: Dialect, limits: Limits) -> Self {
        Self { input, dialect, limits, offset: 0, line: 1, current: None, line_buf: Vec::new(), ready: VecDeque::new(), error: None, finished: false }
    }

    /// Reads the next line into `line_buf`, without its line break, reading at most one byte
    /// more than a record may hold. Returns `None` at the end of the input.
    fn read_line(&mut self) -> Result<Option<Line>, FrameError> {
        self.line_buf.clear();
        // Content plus a CR LF; one byte more than that shows the line is over the limit.
        let cap = self.limits.max_record_bytes.saturating_add(3) as u64;
        let read = (&mut self.input).take(cap).read_until(b'\n', &mut self.line_buf)?;
        if read == 0 {
            return Ok(None);
        }
        let offset = self.offset;
        let number = self.line;
        self.offset += read as u64;
        let line_break: &'static [u8] = if self.line_buf.last() == Some(&b'\n') {
            self.line_buf.pop();
            self.line += 1;
            if self.line_buf.last() == Some(&b'\r') {
                self.line_buf.pop();
                b"\r\n"
            } else {
                b"\n"
            }
        } else {
            b""
        };
        if self.line_buf.len() > self.limits.max_record_bytes {
            return Err(FrameError::RecordTooLarge { record_offset: offset, record_line: number, limit: self.limits.max_record_bytes });
        }
        Ok(Some(Line { offset, number, len: self.line_buf.len(), line_break }))
    }

    /// Moves the record being assembled, if any, to the ready queue.
    fn close_current(&mut self) {
        if let Some(builder) = self.current.take() {
            self.ready.push_back(builder.record);
        }
    }

    /// Frames one line: continues the open record or starts new ones, and parses its properties.
    fn frame_line(&mut self, content: &[u8], line: &Line) -> Result<(), FrameError> {
        let continues = self.current.as_ref().is_some_and(|builder| builder.open_multi_line) && !self.dialect.starts_with_kind(content);
        if content.is_empty() && !continues {
            // A blank line between records.
            return Ok(());
        }
        if line.line_break.is_empty() {
            // The last line has no line break: the file was cut inside a record.
            let (record_offset, record_line) = match (&self.current, continues) {
                (Some(builder), true) => (builder.record.offset, builder.record.line),
                _ => {
                    self.close_current();
                    (line.offset, line.number)
                }
            };
            return Err(FrameError::Truncated { record_offset, record_line, end_offset: self.offset });
        }

        let mut segments = content.split(|byte| *byte == b'|');
        let mut position = 0usize;
        if continues {
            let first = segments.next().unwrap_or_default();
            if let Some(builder) = self.current.as_mut() {
                builder.bytes = builder.bytes.saturating_add(builder.pending_break.len() + content.len());
                if builder.bytes > self.limits.max_record_bytes {
                    return Err(FrameError::RecordTooLarge {
                        record_offset: builder.record.offset,
                        record_line: builder.record.line,
                        limit: self.limits.max_record_bytes,
                    });
                }
                if let Some(last) = builder.record.properties.last_mut() {
                    last.value.extend_from_slice(builder.pending_break);
                    last.value.extend_from_slice(first);
                }
            }
            position = first.len() + 1;
        } else {
            self.close_current();
            let mut builder = Builder::new(line.offset, line.number);
            builder.bytes = content.len();
            self.current = Some(builder);
        }

        let mut last_is_multi_line = continues;
        for segment in segments {
            let offset = line.offset + position as u64;
            position += segment.len() + 1;
            if segment.is_empty() {
                // A leading, trailing or doubled `|` separates nothing.
                continue;
            }
            let property = self.parse_property(segment, offset, line.number)?;
            last_is_multi_line = self.dialect.is_multi_line_property(&property.name);
            self.push_property(property, line)?;
        }
        if let Some(builder) = self.current.as_mut() {
            builder.open_multi_line = last_is_multi_line;
            builder.pending_break = line.line_break;
        }
        Ok(())
    }

    /// Splits one `NAME=VALUE` segment, checking the name against the limits.
    fn parse_property(&self, segment: &[u8], offset: u64, line: u64) -> Result<Property, FrameError> {
        let Some(equals) = segment.iter().position(|byte| *byte == b'=') else {
            return Err(FrameError::PropertyWithoutValue { offset, line });
        };
        let (name, value) = (&segment[..equals], &segment[equals + 1..]);
        if name.is_empty() {
            return Err(FrameError::EmptyPropertyName { offset, line });
        }
        if name.len() > self.limits.max_name_bytes {
            return Err(FrameError::PropertyNameTooLong { offset, line, limit: self.limits.max_name_bytes });
        }
        Ok(Property { name: name.to_vec(), value: value.to_vec(), offset, line })
    }

    /// Adds a property to the open record. A second record-kind property closes the record and
    /// starts the next one at that property, for records that share a line.
    fn push_property(&mut self, property: Property, line: &Line) -> Result<(), FrameError> {
        let is_kind = self.dialect.is_kind_property(&property.name);
        let starts_next = is_kind && self.current.as_ref().is_some_and(|builder| builder.record.kind.is_some());
        if starts_next {
            // The new record takes the bytes from this property to the end of the line.
            let taken = usize::try_from(property.offset - line.offset).unwrap_or(line.len);
            self.close_current();
            let mut builder = Builder::new(property.offset, property.line);
            builder.bytes = line.len.saturating_sub(taken);
            self.current = Some(builder);
        }
        let Some(builder) = self.current.as_mut() else {
            return Ok(());
        };
        if builder.record.properties.len() >= self.limits.max_properties {
            return Err(FrameError::TooManyProperties {
                record_offset: builder.record.offset,
                record_line: builder.record.line,
                limit: self.limits.max_properties,
            });
        }
        if is_kind && builder.record.kind.is_none() {
            builder.record.kind = Some(builder.record.properties.len());
        }
        builder.record.properties.push(property);
        Ok(())
    }

    /// Reads lines until at least one record is ready, an error is found or the input ends.
    fn advance(&mut self) -> Result<(), FrameError> {
        while self.ready.is_empty() {
            let Some(line) = self.read_line()? else {
                self.close_current();
                self.finished = true;
                return Ok(());
            };
            let content = std::mem::take(&mut self.line_buf);
            let framed = self.frame_line(&content, &line);
            self.line_buf = content;
            framed?;
        }
        Ok(())
    }
}

impl<R: BufRead> Iterator for Records<R> {
    type Item = Result<Record, FrameError>;

    /// Returns the next complete record, then any error that stopped framing, then `None`.
    fn next(&mut self) -> Option<Self::Item> {
        if self.ready.is_empty() && !self.finished && self.error.is_none() {
            if let Err(error) = self.advance() {
                self.error = Some(error);
                self.finished = true;
            }
        }
        if let Some(record) = self.ready.pop_front() {
            return Some(Ok(record));
        }
        self.error.take().map(Err)
    }
}

#[cfg(test)]
mod tests;
