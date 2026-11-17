//! Parsed integer-kind tokens on report fields use the same typed setup in every codec.

use super::*;
use crate::board::setup::profile::SetupProfile;

struct NumberField {
    path: &'static str,
    kind: &'static str,
    minimum: i64,
    maximum: i64,
    range_reason: &'static str,
}

/// Published integer-unit fields and their own bounds, independently of their Rust type limits.
fn fields() -> Vec<NumberField> {
    [
        ("/thermal_conductivity/0/conductivity_mw_per_m_k", "u32", 1, 10_000_000, "thermal conductivity must be 1 to 10,000,000"),
        ("/thermal_conductivity/0/temperature_milli_c", "i64", -273_149, 1_000_000, "report temperature must be above absolute zero"),
        ("/cte/0/cte_ppb_per_k", "i64", -1_000_000, 10_000_000, "thermal expansion must be -1,000,000 to 10,000,000"),
        ("/cte/0/from_milli_c", "i64", -273_149, 1_000_000, "temperature range requires both ends, rising"),
        ("/cte/0/to_milli_c", "i64", -273_149, 1_000_000, "temperature range requires both ends, rising"),
        ("/dielectric_strength/0/breakdown_v", "u32", 1, 10_000_000, "dielectric strength must be 1 to 10,000,000"),
        ("/dielectric_strength/0/specimen_thickness_nm", "i64", 1, 100_000_000, "specimen thickness must be 1 to 100,000,000"),
        ("/cti/0/cti_v", "u32", 1, 1000, "CTI must be 1 to 1000"),
    ]
    .into_iter()
    .map(|(path, kind, minimum, maximum, range_reason)| NumberField { path, kind, minimum, maximum, range_reason })
    .collect()
}

/// The widest rising CTE interval permits testing each endpoint independently; CTI has
/// no group here so the numeric representation is independent of the group consistency check.
fn numeric_candidate(s: &Session, path: &str, value: i64) -> Value {
    let mut c = reported(s);
    c["materials"][0]["cte"][0]["from_milli_c"] = json!(-273_149);
    c["materials"][0]["cte"][0]["to_milli_c"] = json!(1_000_000);
    c["materials"][0]["cti"][0].as_object_mut().unwrap().remove("material_group");
    *c["materials"][0].pointer_mut(path).unwrap() = json!(value);
    c
}

/// Preserve the actual decimal/exponent token in a command-built document rather than
/// allowing Value serialization to replace it with a different numeric spelling.
fn encoded_spelling(mut document: Value, path: &str, spelling: &str) -> Vec<u8> {
    let pointer = format!("/setup/materials/0{path}");
    *document.pointer_mut(&pointer).unwrap() = json!("report-number-token");
    let text = serde_json::to_string(&document).unwrap();
    let marker = "\"report-number-token\"";
    assert_eq!(text.matches(marker).count(), 1);
    text.replace(marker, spelling).into_bytes()
}

/// Capture a profile through its production constructor and encoder, not a handwritten fixture.
fn profile(s: &Session) -> Vec<u8> {
    SetupProfile::new("Synthetic numeric reports".into(), s.project().unwrap().board_setup().unwrap().clone(), Default::default())
        .unwrap()
        .encode()
        .unwrap()
}

/// Integer tokens, including each usable edge, round-trip through commands, native documents
/// and profiles. CTE endpoints must leave room for the other endpoint inside the same range.
#[test]
fn integer_report_tokens_agree_on_command_native_and_profile_paths() {
    for field in fields() {
        let lower = field.minimum + i64::from(field.path == "/cte/0/to_milli_c");
        let upper = field.maximum - i64::from(field.path == "/cte/0/from_milli_c");
        for value in [field.minimum.max(0), lower, upper] {
            let mut s = session();
            let original = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
            let canonical = numeric_candidate(&s, field.path, value);
            apply(&mut s, SET_BOARD_SETUP, json!({"setup":canonical}));
            let inspected = setup(&s);
            assert_eq!(inspected["materials"][0].pointer(field.path).unwrap().as_i64(), Some(value));
            let documents = saved(&s, "pcbcraft-board/25");
            let board_path = persistence::test_board_path(&documents);
            let mut changed = documents.clone();
            changed.insert(board_path, encoded_spelling(board_json(&documents), field.path, &value.to_string()));
            let reopened = persistence::decode_documents(&changed).unwrap();
            assert_eq!(persistence::encode_documents(reopened).unwrap(), documents, "native {}: {value}", field.path);

            let profile_bytes = profile(&s);
            let profile_json: Value = serde_json::from_slice(&profile_bytes).unwrap();
            let decoded = SetupProfile::decode(&encoded_spelling(profile_json, field.path, &value.to_string())).unwrap();
            assert_eq!(decoded.encode().unwrap(), profile_bytes, "profile {}: {value}", field.path);
            s.apply("edit.undo", &json!({})).unwrap();
            assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), original);
        }
    }
}

/// Float-kind tokens (even mathematically integral ones), wrong types, overflow and values
/// outside each field's own range are refused for their reason on all three input paths.
#[test]
fn noninteger_report_tokens_and_bounds_are_refused_on_every_path() {
    for field in fields() {
        let mut s = session();
        let canonical = numeric_candidate(&s, field.path, field.minimum.max(0));
        apply(&mut s, SET_BOARD_SETUP, json!({"setup":canonical}));
        let documents = saved(&s, "pcbcraft-board/25");
        let board_path = persistence::test_board_path(&documents);
        let board = board_json(&documents);
        let profile_json: Value = serde_json::from_slice(&profile(&s)).unwrap();
        s.apply("edit.undo", &json!({})).unwrap();
        let mut rejected: Vec<(String, String)> = ["1.0", "1e0", "-0", "0.5", "1e-400", "1e100"]
            .into_iter()
            .map(|spelling| (spelling.to_owned(), "invalid type: floating point".to_owned()))
            .collect();
        rejected.extend([
            ("18446744073709551615".into(), format!("invalid value: integer `18446744073709551615`, expected {}", field.kind)),
            ((field.minimum - 1).to_string(), field.range_reason.into()),
            ((field.maximum + 1).to_string(), field.range_reason.into()),
            ("\"1\"".into(), format!("invalid type: string \"1\", expected {}", field.kind)),
            ("null".into(), format!("invalid type: null, expected {}", field.kind)),
        ]);
        for (spelling, reason) in rejected {
            let mut c = canonical.clone();
            *c["materials"][0].pointer_mut(field.path).unwrap() = serde_json::from_str(&spelling).unwrap();
            refuse(&mut s, c, &reason);

            let mut changed = documents.clone();
            changed.insert(board_path.clone(), encoded_spelling(board.clone(), field.path, &spelling));
            let error = persistence::decode_documents(&changed).map(|_| ()).unwrap_err();
            assert!(error.contains(&reason), "native {}: {spelling}: {error:?} does not name {reason:?}", field.path);
            let error = SetupProfile::decode(&encoded_spelling(profile_json.clone(), field.path, &spelling)).unwrap_err();
            assert!(error.contains(&reason), "profile {}: {spelling}: {error:?} does not name {reason:?}", field.path);
        }
    }
}

/// Extending the new reports does not change the existing typed Tg and Dk/Df readers.
#[test]
fn existing_tg_and_dielectric_numeric_readers_are_unchanged() {
    let mut s = session();
    let base = reported(&s);
    accept_and_undo(&mut s, &base);
    for (path, value, reason) in [
        ("/tg/0/temperature_milli_c", json!(160125.0), "invalid type: floating point `160125.0`, expected i64"),
        ("/dielectric/0/dk_millionths", json!(4200000.0), "invalid type: floating point `4200000.0`, expected u32"),
        ("/dielectric/0/df_millionths", json!(15000.0), "invalid type: floating point `15000.0`, expected u32"),
    ] {
        let mut c = base.clone();
        *c["materials"][0].pointer_mut(path).unwrap() = value;
        refuse(&mut s, c, reason);
    }
}
