use super::*;

/// Read every item of `text`, with the word the outer list starts with.
fn items(text: &str) -> Result<(String, Vec<Node<'_>>), Error> {
    let (mut reader, head) = Reader::open(text)?;
    let mut found = Vec::new();
    while let Some(item) = reader.next_item()? {
        found.push(item);
    }
    Ok((head.into_owned(), found))
}

#[test]
fn words_numbers_and_nested_lists() {
    let (head, found) =
        items("\u{feff}(kicad_sch (version 20231120)\n (wire (pts (xy 10 20.5) (xy -3 4))) hide (property \"Reference\" \"R1\"))").unwrap();
    assert_eq!(head, "kicad_sch");
    assert_eq!(found.len(), 4);
    assert_eq!((found[0].head(), found[0].number(0)), (Some("version"), Some(20_231_120.0)));
    let points: Vec<_> = found[1].child("pts").unwrap().all("xy").map(|p| (p.number(0), p.number(1))).collect();
    assert_eq!(points, [(Some(10.0), Some(20.5)), (Some(-3.0), Some(4.0))]);
    assert_eq!(found[2].word(), Some("hide"));
    assert_eq!((found[3].text(0), found[3].text(1)), (Some("Reference"), Some("R1")));
    // `flag` looks for bare-word arguments; "R1" is one, the list's own head is not.
    assert!(found[3].child("missing").is_none() && found[3].flag("R1") && !found[3].flag("property"));
}

#[test]
fn quoted_words_unescape_and_never_become_lists() {
    let (_, found) = items(r#"(x "say \"hi\"" "back\\slash" "line\nbreak" "(no list)" "" "ü\t")"#).unwrap();
    let words: Vec<_> = found.iter().filter_map(Node::word).collect();
    assert_eq!(words, ["say \"hi\"", "back\\slash", "line\nbreak", "(no list)", "", "ü\t"]);
    // A quoted word is still a word for `flag`, so `(hide "yes")` and `(hide yes)` read alike.
    let (_, found) = items(r#"(x (hide "yes"))"#).unwrap();
    assert!(found[0].flag("yes"));
}

#[test]
fn only_finite_numbers_are_numbers() {
    let (_, found) = items("(x (n 1.25 -0 nan inf -inf 1e400 twelve))").unwrap();
    let numbers: Vec<_> = (0..7).map(|i| found[0].number(i)).collect();
    assert_eq!(numbers, [Some(1.25), Some(-0.0), None, None, None, None, None]);
}

#[test]
fn malformed_text_reports_the_line() {
    for (text, line) in [
        ("", 1),
        ("kicad_sch", 1),
        ("( )", 1),
        ("(kicad_sch\n(wire\n", 3),
        ("(x \"unclosed", 1),
        ("(x \"escape at end\\", 1),
        ("(x) trailing", 1),
        ("(x\n(y)\n)\n\n(z)", 5),
    ] {
        match items(text) {
            Err(Error::Syntax { line: found, .. }) => assert_eq!(found, line, "{text:?}"),
            other => panic!("{text:?} gave {other:?}"),
        }
    }
}

#[test]
fn nesting_and_item_size_are_bounded_but_item_count_is_not() {
    let deep = format!("(x {}", "(".repeat(MAX_DEPTH + 10));
    assert!(matches!(items(&deep), Err(Error::Syntax { message, .. }) if message.contains("nested")));
    let wide = format!("(x (y {}))", "a ".repeat(MAX_NODES));
    assert!(matches!(items(&wide), Err(Error::Syntax { message, .. }) if message.contains("too large")));
    let many = format!("(x {})", "(y 1) ".repeat(50_000));
    assert_eq!(items(&many).unwrap().1.len(), 50_000);
}
