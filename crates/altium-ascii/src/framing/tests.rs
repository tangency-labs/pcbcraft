#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::BufReader;

use super::{Dialect, FrameError, Limits, Record, Records};

/// The dialect of the synthetic files below: records named by `KIND`, with multi-line `TEXT`.
fn dialect() -> Dialect {
    Dialect::new().with_kind_property("KIND").with_multi_line_property("TEXT")
}

/// Frames `text` with the default limits, collecting records until the first error.
fn frame(text: &[u8]) -> (Vec<Record>, Option<FrameError>) {
    frame_with(text, Limits::default())
}

/// Frames `text` with the given limits, collecting records until the first error.
fn frame_with(text: &[u8], limits: Limits) -> (Vec<Record>, Option<FrameError>) {
    let mut records = Vec::new();
    for item in Records::new(text, dialect(), limits) {
        match item {
            Ok(record) => records.push(record),
            Err(error) => return (records, Some(error)),
        }
    }
    (records, None)
}

/// The names and values of a record's properties, as text, for compact assertions.
fn pairs(record: &Record) -> Vec<(String, String)> {
    record
        .properties()
        .iter()
        .map(|property| (String::from_utf8_lossy(property.name()).into_owned(), String::from_utf8_lossy(property.value()).into_owned()))
        .collect()
}

/// Owned copies of name and value pairs, to compare with [`pairs`].
fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
}

const THREE_RECORDS: &[u8] = b"|KIND=Board|VERSION=5.01\r\n\
|KIND=Track|LAYER=TOP|X1=1000mil|WIDTH=10mil\r\n\
|KIND=Text|TEXT=R1|HEIGHT=40mil\r\n";

/// Three records split into three records with their properties.
#[test]
fn three_records_split_into_three_records_with_their_properties() {
    let (records, error) = frame(THREE_RECORDS);
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records.len(), 3);
    assert_eq!(pairs(&records[0]), owned(&[("KIND", "Board"), ("VERSION", "5.01")]));
    assert_eq!(pairs(&records[1]), owned(&[("KIND", "Track"), ("LAYER", "TOP"), ("X1", "1000mil"), ("WIDTH", "10mil"),]));
    assert_eq!(pairs(&records[2]), owned(&[("KIND", "Text"), ("TEXT", "R1"), ("HEIGHT", "40mil")]));
    let starts: Vec<_> = records.iter().map(|record| (record.offset(), record.line())).collect();
    assert_eq!(starts, vec![(0, 1), (26, 2), (72, 3)]);
    assert_eq!(records[1].kind(), Some(&b"Track"[..]));
    assert_eq!(records[1].value("width"), Some(&b"10mil"[..]));
    assert_eq!(records[1].properties()[2].offset(), 48);
}

/// A truncated record is reported with its offset and line.
#[test]
fn a_truncated_record_is_reported_with_its_offset_and_line() {
    let cut = &THREE_RECORDS[..THREE_RECORDS.len() - 8];
    let (records, error) = frame(cut);
    assert_eq!(records.len(), 2, "the complete records come first");
    match error {
        Some(FrameError::Truncated { record_offset, record_line, end_offset }) => {
            assert_eq!((record_offset, record_line), (72, 3));
            assert_eq!(end_offset, cut.len() as u64);
        }
        other => panic!("expected a truncated record, got {other:?}"),
    }
}

/// A file cut inside a multi line text names the record it cut.
#[test]
fn a_file_cut_inside_a_multi_line_text_names_the_record_it_cut() {
    let (records, error) = frame(b"|KIND=Pad|X=1mil\r\n|KIND=Text|TEXT=first\r\nsecond");
    assert_eq!(records.len(), 1);
    match error {
        Some(FrameError::Truncated { record_offset, record_line, .. }) => assert_eq!((record_offset, record_line), (18, 2)),
        other => panic!("expected a truncated record, got {other:?}"),
    }
}

/// A text value keeps its raw line breaks and later properties.
#[test]
fn a_text_value_keeps_its_raw_line_breaks_and_later_properties() {
    let text = b"|KIND=Text|TEXT=Fabrication notes\r\n1. Copper 35 um\r\n\r\n3. Finish ENIG|HEIGHT=40mil|ROTATION=0\r\n\
|KIND=Track|WIDTH=10mil\r\n";
    let (records, error) = frame(text);
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].value("TEXT"), Some(&b"Fabrication notes\r\n1. Copper 35 um\r\n\r\n3. Finish ENIG"[..]));
    assert_eq!(records[0].value("HEIGHT"), Some(&b"40mil"[..]));
    assert_eq!(records[0].value("ROTATION"), Some(&b"0"[..]));
    let height = records[0].property("HEIGHT").unwrap();
    assert_eq!(height.line(), 4);
    assert_eq!(records[1].line(), 5);
    assert_eq!(records[1].kind(), Some(&b"Track"[..]));
}

/// Records that start a line without a leading bar are framed.
#[test]
fn records_that_start_a_line_without_a_leading_bar_are_framed() {
    let text = b"KIND=Board|VERSION=5.01\r\n\
|KIND=Text|TEXT=one line\r\n\
KIND=Track|WIDTH=10mil\r\n\
KIND=Pad|X=1mil\r\n";
    let (records, error) = frame(text);
    assert!(error.is_none(), "{error:?}");
    let kinds: Vec<_> = records.iter().map(Record::kind).collect();
    assert_eq!(kinds, vec![Some(&b"Board"[..]), Some(&b"Text"[..]), Some(&b"Track"[..]), Some(&b"Pad"[..]),]);
    assert_eq!(records[1].value("TEXT"), Some(&b"one line"[..]));
    assert_eq!(records[2].offset(), 51);
}

/// A line that does not continue an open text starts its own record.
#[test]
fn a_line_that_does_not_continue_an_open_text_starts_its_own_record() {
    let (records, error) = frame(b"|KIND=Pad|X=1mil\r\n|X=2mil|Y=3mil\r\n");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].kind(), None);
    assert_eq!(pairs(&records[1]), owned(&[("X", "2mil"), ("Y", "3mil")]));
}

/// Records sharing one line are split at the second kind.
#[test]
fn records_sharing_one_line_are_split_at_the_second_kind() {
    let (records, error) = frame(b"|KIND=Track|WIDTH=10mil|KIND=Pad|X=1mil\r\n");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records.len(), 2);
    assert_eq!(pairs(&records[0]), owned(&[("KIND", "Track"), ("WIDTH", "10mil")]));
    assert_eq!(pairs(&records[1]), owned(&[("KIND", "Pad"), ("X", "1mil")]));
    assert_eq!(records[1].offset(), 24);
}

/// LF line breaks and blank lines between records are accepted.
#[test]
fn lf_line_breaks_and_blank_lines_between_records_are_accepted() {
    let (records, error) = frame(b"|KIND=Pad|X=1mil\n\n|KIND=Text|TEXT=a\nb|H=1mil\n");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].value("TEXT"), Some(&b"a\nb"[..]));
    assert_eq!(records[1].line(), 3);
}

/// A property without equals is reported at its offset.
#[test]
fn a_property_without_equals_is_reported_at_its_offset() {
    let (records, error) = frame(b"|KIND=Pad|X=1mil\r\n|KIND=Track|WIDTH\r\n");
    assert_eq!(records.len(), 1);
    match error {
        Some(FrameError::PropertyWithoutValue { offset, line }) => {
            assert_eq!((offset, line), (30, 2));
        }
        other => panic!("expected a property without value, got {other:?}"),
    }
}

/// A property with an empty name is refused.
#[test]
fn a_property_with_an_empty_name_is_refused() {
    let (_, error) = frame(b"|KIND=Pad|=1mil\r\n");
    assert!(matches!(error, Some(FrameError::EmptyPropertyName { offset: 10, line: 1 })), "{error:?}");
}

/// Framing stops after the first error.
#[test]
fn framing_stops_after_the_first_error() {
    let mut records = Records::new(&b"|KIND=Pad|BROKEN\r\n|KIND=Pad|X=1mil\r\n"[..], dialect(), Limits::default());
    assert!(matches!(records.next(), Some(Err(FrameError::PropertyWithoutValue { .. }))));
    assert!(records.next().is_none());
    assert!(records.next().is_none());
}

/// Record size limit accepts the limit and refuses one byte more.
#[test]
fn record_size_limit_accepts_the_limit_and_refuses_one_byte_more() {
    let line = b"|KIND=Pad|X=1mil\r\n";
    let content = line.len() - 2;
    let at_limit = Limits { max_record_bytes: content, ..Limits::default() };
    let (records, error) = frame_with(line, at_limit);
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records.len(), 1);

    let below = Limits { max_record_bytes: content - 1, ..Limits::default() };
    let (records, error) = frame_with(line, below);
    assert!(records.is_empty());
    assert!(matches!(error, Some(FrameError::RecordTooLarge { record_offset: 0, record_line: 1, .. })), "{error:?}");
}

/// Record size limit counts every line of a multi line text.
#[test]
fn record_size_limit_counts_every_line_of_a_multi_line_text() {
    let text = b"|KIND=Text|TEXT=aaaa\r\nbbbb\r\n";
    // 20 bytes on the first line, then a CR LF and 4 more bytes in the value.
    let at_limit = Limits { max_record_bytes: 26, ..Limits::default() };
    let (records, error) = frame_with(text, at_limit);
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records[0].value("TEXT"), Some(&b"aaaa\r\nbbbb"[..]));

    let below = Limits { max_record_bytes: 25, ..Limits::default() };
    let (_, error) = frame_with(text, below);
    assert!(matches!(error, Some(FrameError::RecordTooLarge { record_offset: 0, .. })), "{error:?}");
}

#[test]
fn same_line_records_get_separate_byte_and_property_budgets() {
    let limits = Limits { max_record_bytes: 8, max_properties: 1, ..Limits::default() };
    let text = b"|KIND=A|kind=B|KiNd=C\r\n";
    // Force both the kind prefix and CR LF across buffer boundaries.
    let records: Vec<_> = Records::new(BufReader::with_capacity(1, &text[..]), dialect(), limits).collect::<Result<_, _>>().unwrap();
    let starts: Vec<_> = records.iter().map(|record| (record.offset(), record.line(), record.kind())).collect();
    assert_eq!(starts, [(0, 1, Some(&b"A"[..])), (8, 1, Some(&b"B"[..])), (15, 1, Some(&b"C"[..]))]);

    let below = Limits { max_record_bytes: 7, ..limits };
    let (records, error) = frame_with(text, below);
    assert!(records.is_empty());
    assert!(matches!(error, Some(FrameError::RecordTooLarge { record_offset: 0, record_line: 1, limit: 7 })));
}

#[test]
fn multiline_budget_stops_at_the_next_same_line_kind() {
    // First record: 14 bytes + 2-byte internal break + `y|` = 18 bytes.
    let text = b"|KIND=A|TEXT=x\r\ny|KIND=B\r\n";
    let limits = Limits { max_record_bytes: 18, ..Limits::default() };
    let (records, error) = frame_with(text, limits);
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].value("TEXT"), Some(&b"x\r\ny"[..]));
    assert_eq!((records[1].offset(), records[1].line()), (18, 2));
    assert_eq!(records[1].kind(), Some(&b"B"[..]));

    let (records, error) = frame_with(text, Limits { max_record_bytes: 17, ..limits });
    assert!(records.is_empty());
    assert!(matches!(error, Some(FrameError::RecordTooLarge { record_offset: 0, record_line: 1, limit: 17 })));
}

#[test]
fn complete_record_precedes_oversized_record_on_the_next_line() {
    // A kind prefix on the next line closes the open text, regardless of that line's length.
    let limits = Limits { max_record_bytes: 14, ..Limits::default() };
    for text in [&b"|KIND=A|TEXT=x\n|KIND=abcdefghijklmnop\n"[..], &b"|KIND=A|TEXT=x\nkind=abcdefghijklmnop\n"[..]] {
        let (records, error) = frame_with(text, limits);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value("TEXT"), Some(&b"x"[..]));
        assert!(matches!(error, Some(FrameError::RecordTooLarge { record_offset: 15, record_line: 2, limit: 14 })));
    }
}

#[test]
fn oversized_continuation_names_the_open_record_start() {
    let (records, error) = frame_with(b"|KIND=P\n|KIND=A|TEXT=x\nabcdefghijklmnopq\n", Limits { max_record_bytes: 15, ..Limits::default() });
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind(), Some(&b"P"[..]));
    assert!(matches!(error, Some(FrameError::RecordTooLarge { record_offset: 8, record_line: 2, limit: 15 })));
}

#[test]
fn same_line_boundary_yields_before_reading_the_later_value() {
    let text = b"|KIND=A|KIND=oversized|KIND=C\r\n";
    let limits = Limits { max_record_bytes: 8, ..Limits::default() };
    let mut records = Records::new(&text[..], dialect(), limits);
    assert_eq!(records.next().unwrap().unwrap().kind(), Some(&b"A"[..]));
    assert_eq!(records.input.reader, b"oversized|KIND=C\r\n");
    assert!(matches!(records.next(), Some(Err(FrameError::RecordTooLarge { record_offset: 8, record_line: 1, limit: 8 }))));
    assert!(records.next().is_none());
}

#[test]
fn same_line_truncation_does_not_hide_a_complete_record() {
    let text = b"|KIND=A|KIND=B";
    let (records, error) = frame(text);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind(), Some(&b"A"[..]));
    assert!(matches!(error, Some(FrameError::Truncated { record_offset: 8, record_line: 1, end_offset: 14 })));
}

#[test]
fn excess_property_value_is_not_read() {
    let limits = Limits { max_properties: 1, ..Limits::default() };
    let mut records = Records::new(&b"|KIND=A|X=unread\n"[..], dialect(), limits);
    assert!(matches!(records.next(), Some(Err(FrameError::TooManyProperties { record_offset: 0, record_line: 1, limit: 1 }))));
    assert_eq!(records.input.reader, b"=unread\n");
    assert!(records.next().is_none());
}

/// Property count limit accepts the limit and refuses one more.
#[test]
fn property_count_limit_accepts_the_limit_and_refuses_one_more() {
    let line = b"|KIND=Pad|X=1mil|Y=2mil\r\n";
    let at_limit = Limits { max_properties: 3, ..Limits::default() };
    let (records, error) = frame_with(line, at_limit);
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records[0].properties().len(), 3);

    let below = Limits { max_properties: 2, ..Limits::default() };
    let (_, error) = frame_with(line, below);
    assert!(matches!(error, Some(FrameError::TooManyProperties { limit: 2, .. })), "{error:?}");
}

/// Property name limit accepts the limit and refuses one byte more.
#[test]
fn property_name_limit_accepts_the_limit_and_refuses_one_byte_more() {
    let line = b"|KIND=Pad|WIDTH=10mil\r\n";
    let at_limit = Limits { max_name_bytes: 5, ..Limits::default() };
    let (records, error) = frame_with(line, at_limit);
    assert!(error.is_none(), "{error:?}");
    assert_eq!(records.len(), 1);

    let below = Limits { max_name_bytes: 4, ..Limits::default() };
    let (_, error) = frame_with(line, below);
    assert!(matches!(error, Some(FrameError::PropertyNameTooLong { offset: 10, line: 1, limit: 4 })), "{error:?}");
}

/// Error messages name the offset and line.
#[test]
fn error_messages_name_the_offset_and_line() {
    let error = FrameError::Truncated { record_offset: 72, record_line: 3, end_offset: 90 };
    assert_eq!(error.to_string(), "the file ends at byte 90 inside the record that starts at byte 72 (line 3)");
}
