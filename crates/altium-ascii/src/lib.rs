//! A bounded, read-only reader for Altium's plain-text (ASCII) board and schematic saves.
//!
//! The reader never writes its input and never panics on damaged data: every limit is checked
//! before the memory it bounds is allocated, and a truncated or malformed record is reported
//! with its byte offset and line instead of being read past.
//!
//! [`framing`] splits a file into records and their `NAME=VALUE` properties. Values stay raw
//! bytes at this level; decoding them into lengths, angles and other values is a separate step.
//!
//! ```
//! use pcbcraft_altium_ascii::framing::{Dialect, Limits, Records};
//!
//! let text = b"|KIND=Track|WIDTH=10mil\r\nKIND=Text|TEXT=two\r\nlines|HEIGHT=40mil\r\n";
//! let dialect = Dialect::new().with_kind_property("KIND").with_multi_line_property("TEXT");
//! let records: Vec<_> = Records::new(&text[..], dialect, Limits::default())
//!     .collect::<Result<_, _>>()
//!     .unwrap_or_default();
//! assert_eq!(records.len(), 2);
//! assert_eq!(records[1].kind(), Some(&b"Text"[..]));
//! assert_eq!(records[1].value("TEXT"), Some(&b"two\r\nlines"[..]));
//! ```

pub mod framing;
