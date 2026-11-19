//! Test the published netlist schemas without converting JSON numbers to floating point.

use serde_json::{Value, value::RawValue};
use std::{cmp::Ordering, collections::BTreeMap};

const UUID_PATTERN: &str = "^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$";

/// Check the hex groups independently of the netlist UUID reader.
fn is_hyphenated_uuid(text: &str) -> bool {
    text.len() == 36 && text.bytes().enumerate().all(|(i, byte)| if matches!(i, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() })
}

/// Decode a node without losing the path of an invalid key or string.
fn decode<'a, T: serde::Deserialize<'a>>(raw: &'a RawValue, at: &str) -> Result<T, String> {
    serde_json::from_str(raw.get()).map_err(|error| format!("{at}: {error}"))
}

/// Significant decimal digits and their power of ten; zero has no sign or scale.
struct Decimal {
    negative: bool,
    digits: String,
    scale: i128,
}

impl Decimal {
    /// Normalize a syntactically valid JSON number. Saturated exponents retain their ordering
    /// against the finite u64 bounds used by these schemas, without expanding powers of ten.
    fn read(text: &str) -> Self {
        let negative = text.starts_with('-');
        let text = text.trim_start_matches('-');
        let (coefficient, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
        let fractional = coefficient.split_once('.').map_or(0, |(_, fraction)| fraction.len());
        let digits = coefficient.replace('.', "");
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return Self { negative: false, digits: String::new(), scale: 0 };
        }
        let significant = digits.trim_end_matches('0');
        let exponent = exponent.parse::<i128>().unwrap_or(if exponent.starts_with('-') { i128::MIN } else { i128::MAX });
        let scale = exponent.saturating_sub(fractional as i128).saturating_add((digits.len() - significant.len()) as i128);
        Self { negative, digits: significant.to_owned(), scale }
    }

    /// Compare exactly with a nonnegative integral schema bound, including fractional inputs.
    fn compare_bound(&self, bound: u64) -> Ordering {
        if self.digits.is_empty() {
            return 0.cmp(&bound);
        }
        if self.negative {
            return Ordering::Less;
        }
        if bound == 0 {
            return Ordering::Greater;
        }
        let bound = bound.to_string();
        let order = (self.digits.len() as i128).saturating_add(self.scale).cmp(&(bound.len() as i128));
        if order != Ordering::Equal {
            return order;
        }
        let count = self.digits.len().max(bound.len());
        self.digits.bytes().chain(std::iter::repeat(b'0')).take(count).cmp(bound.bytes().chain(std::iter::repeat(b'0')).take(count))
    }
}

/// A JSON instance whose object and array children retain their raw numeric tokens.
struct Instance<'a> {
    raw: &'a RawValue,
    number: Option<Decimal>,
    object: Option<BTreeMap<String, Box<RawValue>>>,
    array: Option<Vec<Box<RawValue>>>,
    scalar: Option<Value>,
}

impl<'a> Instance<'a> {
    /// Decode this node; container children keep their raw tokens until visited.
    fn read(raw: &'a RawValue, at: &str) -> Result<Self, String> {
        let first = raw.get().as_bytes()[0];
        Ok(Self {
            raw,
            number: matches!(first, b'-' | b'0'..=b'9').then(|| Decimal::read(raw.get())),
            object: (first == b'{').then(|| decode(raw, at)).transpose()?,
            array: (first == b'[').then(|| decode(raw, at)).transpose()?,
            scalar: matches!(first, b'"' | b'f' | b't' | b'n').then(|| decode(raw, at)).transpose()?,
        })
    }

    /// Reject invalid Unicode before a negation or alternative can hide a decoding error.
    fn check_encoding(&self, at: &str) -> Result<(), String> {
        for (key, child) in self.object.iter().flatten() {
            let at = format!("{at}.{key}");
            Instance::read(child, &at)?.check_encoding(&at)?;
        }
        for (index, child) in self.array.iter().flatten().enumerate() {
            let at = format!("{at}[{index}]");
            Instance::read(child, &at)?.check_encoding(&at)?;
        }
        Ok(())
    }

    /// JSON Schema's mathematical type, including decimal and exponent integer spellings.
    fn kind(&self) -> &'static str {
        if let Some(number) = &self.number {
            return if number.scale >= 0 { "integer" } else { "number" };
        }
        let first = self.raw.get().as_bytes()[0];
        [(b'n', "null"), (b't', "boolean"), (b'f', "boolean"), (b'"', "string"), (b'[', "array"), (b'{', "object")]
            .into_iter()
            .find_map(|(start, kind)| (start == first).then_some(kind))
            .expect("a validated nonnumeric JSON value has a recognized leading token")
    }

    /// Compare constants recursively. Numeric schema constants must be finite u64 values;
    /// an unsupported numeric rule fails instead of silently rounding it.
    fn equals(&self, rule: &Value, at: &str) -> Result<bool, String> {
        if let Some(number) = &self.number {
            return match rule {
                Value::Number(bound) => Ok(number.compare_bound(bound.as_u64().ok_or("unsupported numeric schema constant")?) == Ordering::Equal),
                _ => Ok(false),
            };
        }
        match rule {
            Value::Object(properties) => {
                let Some(object) = &self.object else { return Ok(false) };
                if object.len() != properties.len() {
                    return Ok(false);
                }
                for (key, value) in properties {
                    let Some(child) = object.get(key) else { return Ok(false) };
                    let at = format!("{at}.{key}");
                    if !Instance::read(child, &at)?.equals(value, &at)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::Array(items) => {
                let Some(array) = &self.array else { return Ok(false) };
                if array.len() != items.len() {
                    return Ok(false);
                }
                for (index, (child, rule)) in array.iter().zip(items).enumerate() {
                    let at = format!("{at}[{index}]");
                    if !Instance::read(child, &at)?.equals(rule, &at)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(self.scalar.as_ref() == Some(rule)),
        }
    }
}

/// Validate raw JSON against the keyword forms used by the published schemas.
pub(super) fn check_text(root: &Value, schema: &Value, text: &str, at: &str) -> Result<(), String> {
    supported(root, root)?;
    supported(root, schema)?;
    let raw: Box<RawValue> = serde_json::from_str(text).map_err(|error| format!("{at}: {error}"))?;
    let instance = Instance::read(&raw, at)?;
    instance.check_encoding(at)?;
    check(root, schema, &instance, at)
}

/// Check the unsigned numeric domain of schema constants, including nested containers,
/// before alternatives or negations can turn an unsupported rule into an instance mismatch.
fn supported_constant(value: &Value) -> Result<(), String> {
    match value {
        Value::Number(number) => {
            number.as_u64().ok_or("unsupported numeric schema constant")?;
        }
        Value::Object(properties) => {
            for child in properties.values() {
                supported_constant(child)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                supported_constant(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Resolve a supported local JSON Pointer to a schema object.
fn reference<'a>(root: &'a Value, rule: &Value) -> Result<&'a Value, String> {
    let pointer = rule
        .as_str()
        .and_then(|text| text.strip_prefix('#'))
        .filter(|pointer| pointer.starts_with('/'))
        .ok_or("only local JSON Pointer references are supported")?;
    let target = root.pointer(pointer).ok_or_else(|| format!("unresolved schema reference #{pointer}"))?;
    if !target.is_object() {
        return Err(format!("schema reference #{pointer} does not target an object"));
    }
    Ok(target)
}

/// Reject unknown keywords, invalid references and unsupported numeric rules before matching
/// alternatives or negations, where schema errors could look like instance mismatches.
fn supported(root: &Value, schema: &Value) -> Result<(), String> {
    let schema = schema.as_object().ok_or("unsupported non-object schema")?;
    for (keyword, rule) in schema {
        match keyword.as_str() {
            "$schema" | "$id" | "title" | "description" | "type" | "maxItems" | "required" => {}
            "$ref" => {
                reference(root, rule)?;
            }
            "const" => supported_constant(rule)?,
            "enum" => {
                for value in rule.as_array().ok_or("unsupported schema enum")? {
                    supported_constant(value)?;
                }
            }
            "minimum" | "maximum" | "maxLength" => {
                rule.as_u64().ok_or("unsupported numeric schema bound")?;
            }
            "$defs" | "properties" => {
                for child in rule.as_object().ok_or("unsupported schema map")?.values() {
                    supported(root, child)?;
                }
            }
            "items" | "not" => supported(root, rule)?,
            "anyOf" => {
                for child in rule.as_array().ok_or("unsupported schema alternatives")? {
                    supported(root, child)?;
                }
            }
            "additionalProperties" if rule == false => {}
            "pattern" if rule == "^[0-9a-f]{64}$" || rule == UUID_PATTERN => {}
            "format" if rule == "uuid" => {}
            other => return Err(format!("unsupported schema keyword {other}")),
        }
    }
    Ok(())
}

/// Walk schema references and instance containers together without losing number spellings.
fn check(root: &Value, schema: &Value, value: &Instance<'_>, at: &str) -> Result<(), String> {
    let schema = schema.as_object().ok_or(format!("{at}: schema is not an object"))?;
    for (keyword, rule) in schema {
        match keyword.as_str() {
            "$schema" | "$id" | "title" | "description" | "$defs" => {}
            "$ref" => {
                check(root, reference(root, rule)?, value, at)?;
            }
            "type" => {
                let allowed = rule.as_array().cloned().unwrap_or_else(|| vec![rule.clone()]);
                let actual = value.kind();
                if !allowed.iter().any(|t| t == actual || (t == "number" && actual == "integer")) {
                    return Err(format!("{at}: {actual} is not {rule}"));
                }
            }
            "const" => {
                if !value.equals(rule, at)? {
                    return Err(format!("{at}: not {rule}"));
                }
            }
            "enum" => {
                let mut found = false;
                for rule in rule.as_array().unwrap() {
                    found |= value.equals(rule, at)?;
                }
                if !found {
                    return Err(format!("{at}: {} not in {rule}", value.raw.get()));
                }
            }
            "minimum" | "maximum" => {
                let bound = rule.as_u64().ok_or(format!("{at}: unsupported numeric schema bound"))?;
                if let Some(number) = &value.number {
                    let order = number.compare_bound(bound);
                    if keyword == "minimum" && order == Ordering::Less {
                        return Err(format!("{at}: below {rule}"));
                    }
                    if keyword == "maximum" && order == Ordering::Greater {
                        return Err(format!("{at}: above {rule}"));
                    }
                }
            }
            "maxItems" => {
                if value.array.as_ref().is_some_and(|items| items.len() as u64 > rule.as_u64().unwrap()) {
                    return Err(format!("{at}: more than {rule} items"));
                }
            }
            "maxLength" => {
                let limit = rule.as_u64().ok_or(format!("{at}: unsupported string length limit"))?;
                if value.scalar.as_ref().and_then(Value::as_str).is_some_and(|text| text.chars().count() as u64 > limit) {
                    return Err(format!("{at}: more than {limit} characters"));
                }
            }
            "pattern" if rule == "^[0-9a-f]{64}$" => {
                if let Some(text) = value.scalar.as_ref().and_then(Value::as_str)
                    && (text.len() != 64 || !text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
                {
                    return Err(format!("{at}: not a lowercase SHA-256"));
                }
            }
            "pattern" | "format" if (keyword == "pattern" && rule == UUID_PATTERN) || (keyword == "format" && rule == "uuid") => {
                if value.scalar.as_ref().and_then(Value::as_str).is_some_and(|text| !is_hyphenated_uuid(text)) {
                    return Err(format!("{at}: not a hyphenated UUID"));
                }
            }
            "required" => {
                for name in rule.as_array().unwrap() {
                    if value.object.as_ref().is_some_and(|object| !object.contains_key(name.as_str().unwrap())) {
                        return Err(format!("{at}: missing {name}"));
                    }
                }
            }
            "properties" => {
                if let Some(object) = &value.object {
                    for (name, property) in rule.as_object().unwrap() {
                        if let Some(child) = object.get(name) {
                            let at = format!("{at}.{name}");
                            check(root, property, &Instance::read(child, &at)?, &at)?;
                        }
                    }
                }
            }
            "additionalProperties" if rule == false => {
                if let Some(object) = &value.object {
                    let known = schema.get("properties").and_then(Value::as_object);
                    if let Some(extra) = object.keys().find(|name| !known.is_some_and(|known| known.contains_key(*name))) {
                        return Err(format!("{at}: unexpected {extra}"));
                    }
                }
            }
            "items" => {
                for (index, item) in value.array.iter().flatten().enumerate() {
                    let at = format!("{at}[{index}]");
                    check(root, rule, &Instance::read(item, &at)?, &at)?;
                }
            }
            "not" => {
                if check(root, rule, value, at).is_ok() {
                    return Err(format!("{at}: matches excluded value"));
                }
            }
            "anyOf" => {
                if !rule.as_array().unwrap().iter().any(|option| check(root, option, value, at).is_ok()) {
                    return Err(format!("{at}: matches no alternative"));
                }
            }
            other => return Err(format!("{at}: unsupported schema keyword {other}")),
        }
    }
    Ok(())
}

/// Scalar and container kinds preserve raw numeric tokens, including nested exponents
/// that cannot be represented by a floating-point JSON value.
#[test]
fn raw_instance_kinds_preserve_numeric_tokens() {
    for (text, kind) in [
        ("null", "null"),
        ("true", "boolean"),
        ("false", "boolean"),
        (r#""text""#, "string"),
        ("[]", "array"),
        ("{}", "object"),
        ("0", "integer"),
        ("1.0", "integer"),
        ("1e-1", "number"),
        ("9007199254740993.0000000001", "number"),
        ("[1e999999999999999999999]", "array"),
        (r#"{"nested":1e999999999999999999999}"#, "object"),
    ] {
        let schema = serde_json::json!({"type":kind});
        check_text(&schema, &schema, text, "$").unwrap_or_else(|error| panic!("{text}: {error}"));
    }
}

/// Invalid Unicode is a path-qualified input error, even in an otherwise ignored child.
#[test]
fn fixture_checker_reports_unpaired_surrogates() {
    let pattern = serde_json::json!({"pattern":"^[0-9a-f]{64}$"});
    let uuid = serde_json::json!({"format":"uuid"});
    let scalar = serde_json::json!({"const":"well formed"});
    let nested = serde_json::json!({"not":{"properties":{"items":{"items":{"type":"integer"}}}}});
    let accepts_object = serde_json::json!({"anyOf":[{"type":"object"},{"properties":{"nested":{"type":"object"}}}]});
    for (rule, text, path) in [
        (&pattern, r#""\uDC00""#, "$"),
        (&uuid, r#""\uD800""#, "$"),
        (&scalar, r#""\uDC00""#, "$"),
        (&nested, r#"{"items":[18446744073709551615.0,"\uDC00"]}"#, "$.items[1]"),
        (&accepts_object, r#"{"nested":{"\uDC00":0}}"#, "$.nested"),
    ] {
        let error = check_text(rule, rule, text, "$").unwrap_err();
        assert!(error.starts_with(&format!("{path}: ")), "{text}: {error}");
    }
    let valid = serde_json::json!({"const":{"😀":"😀"}});
    check_text(&valid, &valid, r#"{"\uD83D\uDE00":"\uD83D\uDE00"}"#, "$").unwrap();
}

/// Adjacent large integers and tiny fractional residues remain distinct at integer bounds.
#[test]
fn raw_numbers_preserve_integer_and_bound_semantics() {
    let schema = serde_json::json!({"type":"integer","minimum":0,"maximum":u64::MAX});
    for text in ["0", "-0", "1.0", "1e0", "9007199254740993", "18446744073709551615", "18446744073709551615.0"] {
        check_text(&schema, &schema, text, "$").unwrap_or_else(|error| panic!("{text}: {error}"));
    }
    for (text, reason) in [
        ("18446744073709551616", "above 18446744073709551615"),
        ("18446744073709551615.0000000001", "above 18446744073709551615"),
        ("9007199254740993.0000000001", "number is not \"integer\""),
        ("0.9999999999999999999", "number is not \"integer\""),
        ("-0.0000000000000000001", "below 0"),
    ] {
        let error = check_text(&schema, &schema, text, "$").unwrap_err();
        assert!(error.ends_with(reason), "{text}: {error}");
    }
    let fractional = serde_json::json!({"not":{"type":"integer"}});
    assert!(check_text(&fractional, &fractional, "1.0000000000000000001", "$").is_ok());
    assert_eq!(check_text(&fractional, &fractional, "1.0", "$"), Err("$: matches excluded value".to_owned()));
}

/// An unsupported keyword cannot disappear inside a nonmatching alternative or negation.
#[test]
fn unsupported_rules_are_not_instance_mismatches() {
    for schema in [
        serde_json::json!({"not":{"multipleOf":2}}),
        serde_json::json!({"anyOf":[{"type":"integer"},{"multipleOf":2}]}),
        serde_json::json!({"$defs":{"unused":{"multipleOf":2}},"type":"integer"}),
    ] {
        assert_eq!(check_text(&schema, &schema, "1", "$"), Err("unsupported schema keyword multipleOf".to_owned()));
    }
}

/// A missing or malformed reference is a schema error, not a false alternative.
#[test]
fn fixture_checker_preflights_local_references() {
    let missing = serde_json::json!({"$ref":"#/$defs/missing"});
    for schema in [
        serde_json::json!({"not":missing}),
        serde_json::json!({"anyOf":[{"type":"integer"},missing]}),
        serde_json::json!({"$defs":{"unused":missing},"type":"integer"}),
    ] {
        assert_eq!(check_text(&schema, &schema, "1", "$"), Err("unresolved schema reference #/$defs/missing".to_owned()));
    }
    let bad_target = serde_json::json!({"title":"not a schema", "not":{"$ref":"#/title"}});
    assert_eq!(check_text(&bad_target, &bad_target, "1", "$"), Err("schema reference #/title does not target an object".to_owned()));
    let root = serde_json::json!({"$defs":{"integer":{"type":"integer"}}});
    let valid = serde_json::json!({"not":{"$ref":"#/$defs/integer"}});
    check_text(&root, &valid, "1e-1", "$").unwrap();
    assert_eq!(check_text(&root, &valid, "1.0", "$"), Err("$: matches excluded value".to_owned()));
}

/// Format 2 asserts UUID shape even if a validator treats `format` as annotation only.
#[test]
fn board_schema_uuid_pattern_without_format_annotation() {
    let mut uuid = super::schema_v2()["$defs"]["uuid"].clone();
    assert_eq!(uuid.as_object_mut().unwrap().remove("format"), Some(serde_json::json!("uuid")));
    for good in ["abcdef01-2345-6789-abcd-ef0123456789", "ABCDEF01-2345-6789-ABCD-EF0123456789", "aBcDeF01-2345-6789-AbCd-eF0123456789"] {
        check_text(&uuid, &uuid, &serde_json::json!(good).to_string(), "$.board.id").unwrap();
    }
    for bad in ["not-a-uuid", "abcdef0123456789abcdef0123456789", "abcdef01-2345-6789-abcd-ef012345678g"] {
        let result = check_text(&uuid, &uuid, &serde_json::json!(bad).to_string(), "$.board.id");
        assert_eq!(result, Err("$.board.id: not a hyphenated UUID".to_owned()), "{bad:?}");
    }
    let trailing = r#""abcdef01-2345-6789-abcd-ef0123456789\n""#;
    assert_eq!(check_text(&uuid, &uuid, trailing, "$.board.id"), Err("$.board.id: more than 36 characters".to_owned()));
}

/// Unsupported numeric constants fail before matching, including nested const/enum values
/// under negation, a matching alternative or an unused definition.
#[test]
fn unsupported_numeric_constants_are_not_instance_mismatches() {
    for value in [serde_json::json!(-1), serde_json::json!(0.5), serde_json::json!({"nested":[-1]}), serde_json::json!([{"nested":0.5}])] {
        for rule in [serde_json::json!({"const":value}), serde_json::json!({"enum":["other",value]})] {
            for schema in [
                serde_json::json!({"not":rule}),
                serde_json::json!({"anyOf":[{"type":"integer"},rule]}),
                serde_json::json!({"$defs":{"unused":rule},"type":"integer"}),
            ] {
                assert_eq!(check_text(&schema, &schema, "-1", "$"), Err("unsupported numeric schema constant".to_owned()), "{schema}");
            }
        }
    }
}

/// Nested object/array constants retain exact numeric equality through const and enum,
/// including integral exponent spellings above f64's consecutive-integer range.
#[test]
fn supported_numeric_constants_compare_recursively() {
    let value = serde_json::json!({"array":[u64::MAX,{"integer":9_007_199_254_740_993_u64}],"other":[true,null,"text"]});
    let exact = r#"{"array":[18446744073709551615.0,{"integer":9007199254740993e0}],"other":[true,null,"text"]}"#;
    let different = exact.replace("9007199254740993e0", "9007199254740992e0");
    for schema in [serde_json::json!({"const":value}), serde_json::json!({"enum":["other",value]})] {
        check_text(&schema, &schema, exact, "$").unwrap();
        let reason =
            if let Some(rule) = schema.get("const") { format!("$: not {rule}") } else { format!("$: {different} not in {}", schema["enum"]) };
        assert_eq!(check_text(&schema, &schema, &different, "$"), Err(reason));
        let excluded = serde_json::json!({"not":schema});
        assert_eq!(check_text(&excluded, &excluded, exact, "$"), Err("$: matches excluded value".to_owned()));
        check_text(&excluded, &excluded, &different, "$").unwrap();
        let unused = serde_json::json!({"$defs":{"unused":schema},"type":"integer"});
        check_text(&unused, &unused, "0", "$").unwrap();
    }
}
