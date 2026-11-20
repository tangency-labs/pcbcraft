//! Integer token restrictions through shared commands and the board/profile codecs.
use super::*;
use crate::board::{setup::profile::SetupProfile, silkscreen::Settings};

/// Substitute a raw numeric spelling into a command-built fixture for reader probes.
fn spelled(value: &Value, pointer: &str, spelling: &str) -> Vec<u8> {
    let mut marked = value.clone();
    *marked.pointer_mut(pointer).unwrap() = json!("@number@");
    marked.to_string().replace("\"@number@\"", spelling).into_bytes()
}

/// Like existing native integer fields, these fields accept integer-kind numbers only.
/// JSON Schema's integer type also allows integral decimal/exponent values; the reader does not.
/// In particular, an underflowed fractional token must not be adopted as a zero tolerance.
#[test]
fn copper_and_board_integer_tokens_keep_the_native_reader_restriction_on_each_path() {
    let mut s = session(2);
    let mut base = setup(&s);
    base["stackup"]["strata"][0]["thickness_nm"] = json!(50_000_000);
    base["stackup"]["strata"][0]["thickness_tolerance_nm"] = json!(0);
    base["stackup"]["strata"][0]["base_copper"] = json!({"weight_milli_oz":1,"thickness_nm":1});
    base["stackup"]["finished_thickness_nm"] = json!(50_000_000);
    base["stackup"]["finished_thickness_tolerance_nm"] = json!(0);
    base["stackup"]["declared_minimum_thickness_nm"] = json!(1);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":base}));
    let documents = saved_board(&s, "pcbcraft-board/23");
    let board_path = persistence::test_board_path(&documents);
    let board: Value = serde_json::from_slice(&documents[&board_path]).unwrap();
    let profile = SetupProfile::new("Copper example".into(), s.project().unwrap().board_setup().unwrap().clone(), Settings::default()).unwrap();
    let profile: Value = serde_json::from_slice(&profile.encode().unwrap()).unwrap();
    let schema = BoardSetup::schema();
    let stack_schema = &schema["properties"]["stackup"]["properties"];
    let copper_schema = &stack_schema["strata"]["items"]["oneOf"][0]["properties"];
    for (pointer, scalar, minimum, maximum, range_reason) in [
        (
            "/stackup/strata/0/base_copper/weight_milli_oz",
            &copper_schema["base_copper"]["properties"]["weight_milli_oz"],
            1,
            100_000,
            "base copper weight must be",
        ),
        (
            "/stackup/strata/0/base_copper/thickness_nm",
            &copper_schema["base_copper"]["properties"]["thickness_nm"],
            1,
            100_000_000,
            "base copper foil thickness must be",
        ),
        ("/stackup/strata/0/thickness_tolerance_nm", &copper_schema["thickness_tolerance_nm"], 0, 100_000_000, "copper tolerance requires"),
        ("/stackup/finished_thickness_nm", &stack_schema["finished_thickness_nm"], 1, 100_000_000, "finished board thickness must be"),
        (
            "/stackup/finished_thickness_tolerance_nm",
            &stack_schema["finished_thickness_tolerance_nm"],
            0,
            100_000_000,
            "board thickness tolerance requires",
        ),
        (
            "/stackup/declared_minimum_thickness_nm",
            &stack_schema["declared_minimum_thickness_nm"],
            1,
            100_000_000,
            "declared minimum board thickness must be",
        ),
    ] {
        // Check the actual published scalar, not a substitute schema validator. The token-kind
        // restriction is stated separately because JSON Schema does not express it.
        assert_eq!(scalar["type"], "integer");
        assert_eq!(scalar["minimum"], minimum);
        assert_eq!(scalar["maximum"], maximum);
        assert!(scalar["description"].as_str().unwrap().contains("integer-kind"));
        let tolerance = minimum == 0;
        let mut base = base.clone();
        if pointer == "/stackup/declared_minimum_thickness_nm" {
            base["stackup"]["finished_thickness_nm"] = json!(maximum);
        }
        let mut board = board.clone();
        board["setup"] = base.clone();
        let mut profile = profile.clone();
        profile["setup"] = base.clone();
        let last_good = if tolerance { 49_999_999 } else { maximum };
        for integer in [minimum, 1, last_good] {
            let mut expected = base.clone();
            *expected.pointer_mut(pointer).unwrap() = json!(integer);
            apply(&mut s, SET_BOARD_SETUP, json!({"setup":expected}));
            assert_eq!(setup(&s), expected);
            saved_board(&s, "pcbcraft-board/23");
            let mut changed = documents.clone();
            changed.insert(board_path.clone(), spelled(&board, &format!("/setup{pointer}"), &integer.to_string()));
            let opened = persistence::decode_documents(&changed).unwrap();
            assert_eq!(serde_json::to_value(opened.board_setup().unwrap()).unwrap(), expected);
            let opened = SetupProfile::decode(&spelled(&profile, &format!("/setup{pointer}"), &integer.to_string())).unwrap();
            assert_eq!(serde_json::to_value(opened.setup).unwrap(), expected);
        }
        let mut bad = ["1.0", "1e0", "0.0", "0e0", "-0", "-0.0", "-0e0", "1.5", "1e-100", "1e-400", "-1e-400", "1e308"]
            .map(|spelling| (spelling.to_owned(), "floating point"))
            .to_vec();
        bad.push(((maximum + 1).to_string(), range_reason));
        let wide_reason = if maximum == 100_000 { "invalid value: integer" } else { range_reason };
        for integer in [9_007_199_254_740_993_i64, i64::MAX - 1, i64::MAX] {
            bad.push((integer.to_string(), wide_reason));
        }
        bad.push((u64::MAX.to_string(), "invalid value: integer"));
        for (spelling, reason) in bad {
            let mut params: Value = serde_json::from_slice(&spelled(&json!({"setup":base}), &format!("/setup{pointer}"), &spelling)).unwrap();
            params["expected_revision"] = json!(s.revision());
            reject_because(&mut s, SET_BOARD_SETUP, params, reason);
            let mut changed = documents.clone();
            changed.insert(board_path.clone(), spelled(&board, &format!("/setup{pointer}"), &spelling));
            refused_because(&changed, reason);
            let error = SetupProfile::decode(&spelled(&profile, &format!("/setup{pointer}"), &spelling)).unwrap_err();
            assert!(error.contains(reason), "profile: {pointer} = {spelling}: {error}");
        }
    }
}
