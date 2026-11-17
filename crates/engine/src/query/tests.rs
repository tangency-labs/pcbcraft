use super::*;

/// Print tree structure independently of whitespace and grouping spans for the syntax corpus.
fn canonical(expression: &Expression) -> String {
    match &expression.kind {
        ExpressionKind::Constant(value) => value.to_string(),
        ExpressionKind::Not(child) => format!("not({})", canonical(child)),
        ExpressionKind::And(children) | ExpressionKind::Or(children) => {
            let operator = if matches!(expression.kind, ExpressionKind::And(_)) { "and" } else { "or" };
            format!("{operator}({})", children.iter().map(canonical).collect::<Vec<_>>().join(","))
        }
        ExpressionKind::Predicate(Predicate::Text { field, comparison, value }) => {
            format!("{field:?}{comparison:?}{value:?}")
        }
        ExpressionKind::Predicate(Predicate::Number { property, comparison, value }) => {
            format!("property({property:?}){comparison:?}{value}")
        }
    }
}

/// Forty original queries assert trees, including precedence, escapes and exact numeric text.
#[test]
fn forty_valid_queries_have_the_listed_trees() {
    let cases = [
        ("true", "true"),
        ("false", "false"),
        ("  true\n", "true"),
        ("not true", "not(true)"),
        ("!false", "not(false)"),
        ("not not true", "not(not(true))"),
        ("(true)", "true"),
        ("((false))", "false"),
        ("true and false", "and(true,false)"),
        ("true or false", "or(true,false)"),
        ("true && false", "and(true,false)"),
        ("true || false", "or(true,false)"),
        ("true or false and true", "or(true,and(false,true))"),
        ("true and false or true", "or(and(true,false),true)"),
        ("(true or false) and true", "and(or(true,false),true)"),
        ("not (true or false)", "not(or(true,false))"),
        ("true and false and true", "and(true,false,true)"),
        ("false or true or false", "or(false,true,false)"),
        (r#"kind = "track""#, r#"KindEqual"track""#),
        (r#"kind != "via""#, r#"KindNotEqual"via""#),
        (r#"layer = "F.Cu""#, r#"LayerEqual"F.Cu""#),
        (r#"net = "GND""#, r#"NetEqual"GND""#),
        (r#"net_class = "Power""#, r#"NetClassEqual"Power""#),
        (r#"part_reference = "R1""#, r#"PartReferenceEqual"R1""#),
        (r#"part_group = "Analog""#, r#"PartGroupEqual"Analog""#),
        (r#"footprint = "generic-0603""#, r#"FootprintEqual"generic-0603""#),
        (r#"net = "電源""#, r#"NetEqual"電源""#),
        (r#"net = "a\"b""#, r#"NetEqual"a\"b""#),
        (r#"net = "a\\b""#, r#"NetEqual"a\\b""#),
        (r#"net = "\u03a9""#, r#"NetEqual"Ω""#),
        (r#"net = "and || not""#, r#"NetEqual"and || not""#),
        (r#"net != """#, r#"NetNotEqual"""#),
        (r#"property("width_nm") = 0"#, r#"property("width_nm")Equal0"#),
        (r#"property("width_nm") != -1"#, r#"property("width_nm")NotEqual-1"#),
        (r#"property("width_nm") < +002.500"#, r#"property("width_nm")Less2.5"#),
        (r#"property("width_nm") <= 1e3"#, r#"property("width_nm")LessOrEqual1000"#),
        (r#"property("width_nm") > 1E-3"#, r#"property("width_nm")Greater0.001"#),
        (r#"property("width_nm") >= -0.0"#, r#"property("width_nm")GreaterOrEqual0"#),
        (r#"not kind = "pad" && net != "GND" || false"#, r#"or(and(not(KindEqual"pad"),NetNotEqual"GND"),false)"#),
        (
            r#"(property("core:voltage")>=3.3)and(layer="F.Cu" or layer="B.Cu")"#,
            r#"and(property("core:voltage")GreaterOrEqual3.3,or(LayerEqual"F.Cu",LayerEqual"B.Cu"))"#,
        ),
    ];
    assert_eq!(cases.len(), 40);
    for (input, tree) in cases {
        assert_eq!(canonical(&parse(input).unwrap().expression), tree, "{input}");
    }
}

/// Twenty refused queries assert the diagnostic category, byte offset and actual cause.
#[test]
fn twenty_invalid_queries_have_the_listed_diagnostics() {
    use ParseErrorKind::*;
    let cases = [
        ("", UnexpectedToken, 0, "Expected a selector"),
        ("   ", UnexpectedToken, 3, "Expected a selector"),
        ("()", UnexpectedToken, 1, "Expected a selector"),
        ("(true", UnexpectedToken, 5, "Expected ')'"),
        ("true)", UnexpectedToken, 4, "Expected 'and'"),
        ("true and", UnexpectedToken, 8, "Expected a selector"),
        ("or true", UnexpectedToken, 0, "Expected a selector"),
        ("true false", UnexpectedToken, 5, "Expected 'and'"),
        (r#"unknown = "x""#, UnknownField, 0, "Unknown selector"),
        (r#"net "x""#, UnexpectedToken, 4, "Expected a comparison"),
        (r#"net = x"#, UnexpectedToken, 6, "Expected double-quoted"),
        (r#"net < "x""#, InvalidComparison, 4, "Text selectors accept only"),
        (r#"net = "x"#, InvalidString, 6, "Unterminated"),
        (r#"net = "\q""#, InvalidString, 6, "Invalid quoted text"),
        (r#"property "x" = 1"#, UnexpectedToken, 9, "Expected '('"),
        (r#"property("x" = 1"#, UnexpectedToken, 13, "Expected ')'"),
        (r#"property("x") = "1""#, UnexpectedToken, 16, "Expected a decimal"),
        (r#"property("x") = 1e"#, InvalidNumber, 16, "Expected exponent digits"),
        (r#"net = "電源" and )"#, UnexpectedToken, 19, "Expected a selector"),
        ("true & false", UnexpectedToken, 5, "Unexpected character"),
    ];
    assert_eq!(cases.len(), 20);
    for (input, kind, offset, reason) in cases {
        let failure = parse(input).unwrap_err();
        assert_eq!(failure.kind, kind, "{input}: {failure}");
        assert_eq!(failure.offset, offset, "{input}: {failure}");
        assert!(failure.message.starts_with(reason), "{input}: {failure}");
        assert!(input.is_char_boundary(failure.offset));
    }
}

/// Numeric rules retain integers past binary float precision, shared decimal bounds and signs.
#[test]
fn numbers_are_exact_at_the_shared_decimal_edges() {
    for number in [
        "9007199254740993",
        "9223372036854775806",
        "9223372036854775807",
        "18446744073709551615",
        "999999999999999999999999999999999999",
        "-999999999999999999999999999999999999",
        "0.000000000000000001",
        "-0.000000000000000001",
    ] {
        let query = parse(&format!(r#"property("n") = {number}"#)).unwrap();
        let ExpressionKind::Predicate(Predicate::Number { value, .. }) = query.expression.kind else { panic!("numeric predicate") };
        assert_eq!(value, number);
    }
    for number in [
        "1000000000000000000000000000000000000",
        "-1000000000000000000000000000000000000",
        "0.0000000000000000001",
        "-0.0000000000000000001",
        "1e49",
        "0e-49",
        "1.2.3",
        "--1",
        ".5",
        "1.",
        "1e+",
        "1mm",
    ] {
        let failure = parse(&format!(r#"property("n") = {number}"#)).unwrap_err();
        assert_eq!(failure.kind, ParseErrorKind::InvalidNumber, "{number}: {failure}");
        assert_eq!(failure.offset, 16);
    }
    assert_eq!(number_value("0e48"), Ok("0".into()));
    assert_eq!(number_value(&format!("{}1", "0".repeat(127))), Ok("1".into()));
    assert_eq!(number_value(&format!("{}1", "0".repeat(128))), Err("Number exceeds shared decimal precision or exponent limits."));
}

/// The byte, token and combined-depth limits accept the last good input and refuse the next.
#[test]
fn resource_limits_bound_wide_and_deep_queries() {
    let at_bytes = format!("true{}", " ".repeat(MAX_QUERY_BYTES - 4));
    assert_eq!(canonical(&parse(&at_bytes).unwrap().expression), "true");
    assert_eq!(parse(&(at_bytes + " ")).unwrap_err(), error(ParseErrorKind::LimitExceeded, 0, "Query exceeds 16384 UTF-8 bytes."));
    let wide = std::iter::repeat_n("true", MAX_QUERY_TOKENS / 2).collect::<Vec<_>>().join("||");
    let at_tokens = format!("!{wide}");
    let query = parse(&at_tokens).unwrap();
    assert_eq!(query, query.clone());
    let failure = parse(&format!("!!{wide}")).unwrap_err();
    assert_eq!(failure.kind, ParseErrorKind::LimitExceeded);
    assert_eq!(failure.message, "Query exceeds 4096 tokens.");
    assert_eq!(failure.offset, at_tokens.len() - 3);
    for (prefix, suffix) in [("!", ""), ("(", ")")] {
        let at_depth = format!("{}true{}", prefix.repeat(MAX_QUERY_DEPTH), suffix.repeat(MAX_QUERY_DEPTH));
        parse(&at_depth).unwrap();
        let failure = parse(&format!("{prefix}{at_depth}{suffix}")).unwrap_err();
        assert_eq!(failure.kind, ParseErrorKind::LimitExceeded);
        assert_eq!(failure.message, "Query exceeds 32 nested parentheses or negations.");
        assert_eq!(failure.offset, MAX_QUERY_DEPTH);
    }
    let mixed = format!("{}true{}", "(!".repeat(MAX_QUERY_DEPTH / 2), ")".repeat(MAX_QUERY_DEPTH / 2));
    parse(&mixed).unwrap();
    assert_eq!(parse(&format!("!{mixed}")).unwrap_err().kind, ParseErrorKind::LimitExceeded);
}

/// Spans slice the original Unicode input; grouping changes only the outer node's extent.
#[test]
fn unicode_spans_and_json_strings_are_byte_addressed() {
    let input = "\u{2003}(net = \"電源\" or net = \"Ω\") ";
    let query = parse(input).unwrap();
    assert_eq!(&input[query.expression.span.clone()], "(net = \"電源\" or net = \"Ω\")");
    let ExpressionKind::Or(children) = query.expression.kind else { panic!("disjunction") };
    assert_eq!(&input[children[0].span.clone()], "net = \"電源\"");
    assert_eq!(&input[children[1].span.clone()], "net = \"Ω\"");
    for text in [r#""\ud800""#, r#""\udfff""#, "\"a\nb\"", "\"\0\""] {
        let failure = parse(&format!("net={text}")).unwrap_err();
        assert_eq!(failure.kind, ParseErrorKind::InvalidString);
        assert_eq!(failure.offset, 4);
    }
    assert_eq!(canonical(&parse(r#"net="\ud83d\ude00""#).unwrap().expression), "NetEqual\"😀\"");
    let failure = parse("net=\"Ω\"💡").unwrap_err();
    assert_eq!(failure.offset, 8);
    assert_eq!(failure.to_string(), "byte 8: Unexpected character.");
}

/// Each selector and comparison rule is exercised, including unsupported spellings and types.
#[test]
fn selector_and_operator_rules_are_checked_row_by_row() {
    for field in ["kind", "layer", "net", "net_class", "part_reference", "part_group", "footprint"] {
        for operator in ["=", "!="] {
            parse(&format!("{field} {operator} \"x\"")).unwrap();
        }
        for operator in ["<", "<=", ">", ">="] {
            let failure = parse(&format!("{field} {operator} \"x\"")).unwrap_err();
            assert_eq!(failure.kind, ParseErrorKind::InvalidComparison);
            assert!(failure.message.contains("only '=' and '!='"));
        }
        for value in ["null", "1", "true", "", "{}", "[]"] {
            let failure = parse(&format!("{field} = {value}")).unwrap_err();
            assert_eq!(failure.kind, ParseErrorKind::UnexpectedToken);
        }
    }
    for operator in ["=", "!=", "<", "<=", ">", ">="] {
        parse(&format!("property(\"n\") {operator} 1")).unwrap();
    }
    for (input, kind, reason) in [
        (r#"property("") = 1"#, ParseErrorKind::UnexpectedToken, "must not be empty"),
        (r#"net == "x""#, ParseErrorKind::UnexpectedToken, "double-quoted"),
        ("true OR false", ParseErrorKind::UnexpectedToken, "end of query"),
        ("TRUE", ParseErrorKind::UnknownField, "Unknown selector"),
        (r#"Kind="track""#, ParseErrorKind::UnknownField, "Unknown selector"),
        (r#"component="R1""#, ParseErrorKind::UnknownField, "Unknown selector"),
        ("inside(0,0,1,1)", ParseErrorKind::UnknownField, "Unknown selector"),
        (r#"property("n")=true"#, ParseErrorKind::UnexpectedToken, "decimal number"),
    ] {
        let failure = parse(input).unwrap_err();
        assert_eq!(failure.kind, kind, "{input}: {failure}");
        assert!(failure.message.contains(reason), "{input}: {failure}");
    }
}

/// Parsing succeeds or fails without touching the command-created fixture, encoded bytes or undo.
#[test]
fn parsing_is_independent_of_project_state_and_history() {
    use crate::{Session, persistence::encode_documents};
    use serde_json::json;

    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Query syntax"})).unwrap();
    let before = session.snapshot().unwrap();
    let encoded = encode_documents(before.clone()).unwrap();
    let query = parse(r#"kind="track" and net="GND""#).unwrap();
    assert!(matches!(query.expression.kind, ExpressionKind::And(_)));
    let failure = parse(r#"net="Ω" and"#).unwrap_err();
    assert_eq!(failure.kind, ParseErrorKind::UnexpectedToken);
    assert_eq!(failure.offset, 12);
    assert!(failure.message.contains("Expected a selector"));
    assert!(std::sync::Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), encoded);
    session.apply("edit.undo", &serde_json::Value::Null).unwrap();
    assert!(session.snapshot().is_none());
}
