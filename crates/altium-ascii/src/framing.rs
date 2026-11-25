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
//! A record ends at a line break or at the next kind on the same line. A file that ends
//! inside a record, a property without `=`, an empty or over-long property name, and a record
//! over the size or property limits are
//! each reported as a [`FrameError`] with the byte offset and line where it occurs. Iteration
//! stops after the first error, so nothing past a damaged record is read or guessed.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufRead};

/// Limits applied while framing. Each one is checked before memory for the data it bounds is
/// allocated, so a damaged or hostile file cannot make the reader grow without bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest record, in file bytes, including internal line breaks but not the final
    /// line break. At a same-line split, the separator before the next kind belongs to the
    /// preceding record.
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

/// Why framing stopped. Format errors report a byte offset and line; I/O errors wrap the input error.
#[derive(Debug)]
pub enum FrameError {
    /// Reading the input failed.
    Io(io::Error),
    /// The file ends inside a record without a terminating line break or next kind.
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

#[derive(Clone, Copy)]
struct Position {
    offset: u64,
    line: u64,
}

/// A cursor with just enough lookahead to recognize CR LF and `|KIND=` across input buffers.
struct Input<R> {
    reader: R,
    lookahead: VecDeque<u8>,
    position: Position,
}

impl<R: BufRead> Input<R> {
    fn peek(&mut self, index: usize) -> io::Result<Option<u8>> {
        while self.lookahead.len() <= index {
            let bytes = match self.reader.fill_buf() {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            let Some(&byte) = bytes.first() else {
                return Ok(None);
            };
            self.lookahead.push_back(byte);
            self.reader.consume(1);
        }
        Ok(self.lookahead.get(index).copied())
    }

    /// Commits bytes already present in lookahead.
    fn consume(&mut self, count: usize) {
        for _ in 0..count {
            self.lookahead.pop_front();
        }
        self.position.offset += count as u64;
    }

    fn line_break(&mut self) -> io::Result<&'static [u8]> {
        match self.peek(0)? {
            Some(b'\n') => Ok(b"\n"),
            Some(b'\r') if self.peek(1)? == Some(b'\n') => Ok(b"\r\n"),
            _ => Ok(b""),
        }
    }

    fn consume_line_break(&mut self, line_break: &[u8]) {
        self.consume(line_break.len());
        self.position.line += 1;
    }
}

/// An iterator over the records of an ASCII save, read from any buffered input.
///
/// It yields at each record boundary and stops after the first [`FrameError`]. Apart from the
/// caller's input buffer, it keeps only one record and enough bytes to recognize a line break
/// or a record-kind prefix, not a whole physical line.
pub struct Records<R> {
    input: Input<R>,
    dialect: Dialect,
    limits: Limits,
    finished: bool,
}

impl<R: BufRead> Records<R> {
    /// Starts framing `input` with the given dialect and limits.
    pub fn new(input: R, dialect: Dialect, limits: Limits) -> Self {
        let input = Input { reader: input, lookahead: VecDeque::new(), position: Position { offset: 0, line: 1 } };
        Self { input, dialect, limits, finished: false }
    }

    /// Recognizes a kind without consuming it from the parser's cursor or reading its value.
    fn starts_with_kind(&mut self, allow_bar: bool) -> io::Result<bool> {
        let Some(kind) = self.dialect.kind_property().filter(|kind| !kind.is_empty()) else {
            return Ok(false);
        };
        let skip = usize::from(allow_bar && self.input.peek(0)? == Some(b'|'));
        for (index, name_byte) in kind.iter().enumerate() {
            if matches!(name_byte, b'|' | b'\n' | b'=') || !self.input.peek(index + skip)?.is_some_and(|byte| byte.eq_ignore_ascii_case(name_byte)) {
                return Ok(false);
            }
        }
        Ok(self.input.peek(kind.len() + skip)? == Some(b'='))
    }

    fn check_size(&self, start: Position, extra: u64) -> Result<(), FrameError> {
        let used = self.input.position.offset - start.offset;
        if used.saturating_add(extra) > self.limits.max_record_bytes as u64 {
            return Err(FrameError::RecordTooLarge { record_offset: start.offset, record_line: start.line, limit: self.limits.max_record_bytes });
        }
        Ok(())
    }

    fn truncated(&self, start: Position) -> FrameError {
        FrameError::Truncated { record_offset: start.offset, record_line: start.line, end_offset: self.input.position.offset }
    }

    fn read_value(&mut self, start: Position, value: &mut Vec<u8>) -> Result<(), FrameError> {
        while let Some(byte) = self.input.peek(0)? {
            if byte == b'|' || !self.input.line_break()?.is_empty() {
                break;
            }
            self.check_size(start, 1)?;
            self.input.consume(1);
            value.push(byte);
        }
        Ok(())
    }

    fn read_property(&mut self, start: Position) -> Result<Property, FrameError> {
        let Position { offset, line } = self.input.position;
        let mut name = Vec::new();
        loop {
            match self.input.peek(0)? {
                None => return Err(self.truncated(start)),
                Some(b'=') => {
                    if name.is_empty() {
                        return Err(FrameError::EmptyPropertyName { offset, line });
                    }
                    self.check_size(start, 1)?;
                    self.input.consume(1);
                    break;
                }
                Some(byte) => {
                    if byte == b'|' || !self.input.line_break()?.is_empty() {
                        return Err(FrameError::PropertyWithoutValue { offset, line });
                    }
                    if name.len() >= self.limits.max_name_bytes {
                        return Err(FrameError::PropertyNameTooLong { offset, line, limit: self.limits.max_name_bytes });
                    }
                    self.check_size(start, 1)?;
                    self.input.consume(1);
                    name.push(byte);
                }
            }
        }
        let mut value = Vec::new();
        self.read_value(start, &mut value)?;
        Ok(Property { name, value, offset, line })
    }

    fn read_record(&mut self) -> Result<Option<Record>, FrameError> {
        // Blank lines outside a record have no byte cost.
        loop {
            let line_break = self.input.line_break()?;
            if line_break.is_empty() {
                break;
            }
            self.input.consume_line_break(line_break);
        }
        if self.input.peek(0)?.is_none() {
            return Ok(None);
        }

        let start = self.input.position;
        let mut record = Record { offset: start.offset, line: start.line, properties: Vec::new(), kind: None };
        loop {
            let line_break = self.input.line_break()?;
            if !line_break.is_empty() {
                self.input.consume_line_break(line_break);
                if let Some(last) = record.properties.last_mut()
                    && self.dialect.is_multi_line_property(&last.name)
                    && self.input.peek(0)?.is_some()
                    && !self.starts_with_kind(true)?
                {
                    // This break belongs to the value only after a continuation is known.
                    self.check_size(start, 0)?;
                    last.value.extend_from_slice(line_break);
                    self.read_value(start, &mut last.value)?;
                    continue;
                }
                return Ok(Some(record));
            }
            match self.input.peek(0)? {
                None => return Err(self.truncated(start)),
                Some(b'|') => {
                    self.check_size(start, 1)?;
                    self.input.consume(1);
                    continue;
                }
                Some(_) if record.kind.is_some() && self.starts_with_kind(false)? => return Ok(Some(record)),
                Some(_) => {}
            }
            // A new kind above belongs to the next record even if this record is full.
            if record.properties.len() >= self.limits.max_properties {
                return Err(FrameError::TooManyProperties {
                    record_offset: start.offset,
                    record_line: start.line,
                    limit: self.limits.max_properties,
                });
            }
            let property = self.read_property(start)?;
            if self.dialect.is_kind_property(&property.name) {
                record.kind = Some(record.properties.len());
            }
            record.properties.push(property);
        }
    }
}

impl<R: BufRead> Iterator for Records<R> {
    type Item = Result<Record, FrameError>;

    /// Returns the next complete record, then any error that stopped framing, then `None`.
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match self.read_record() {
            Ok(Some(record)) => Some(Ok(record)),
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}

#[cfg(test)]
mod tests;
