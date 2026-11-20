//! String-valued copper enums through commands and the native/profile readers.

use super::*;
use crate::board::{setup::profile::SetupProfile, silkscreen::Settings};

/// Author a valid setup with both string-valued fields through the shared command.
fn specified() -> Session {
    let mut s = session(2);
    let mut candidate = setup(&s);
    candidate["stackup"]["strata"][0]["role"] = json!("signal");
    candidate["stackup"]["strata"][0]["thickness_nm"] = json!(47_900);
    candidate["stackup"]["strata"][0]["thickness_basis"] = json!("nominal");
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
    s
}

/// Every published role and thickness-basis string survives each production reader and history.
#[test]
fn copper_role_and_thickness_basis_strings_round_trip_on_all_paths() {
    let mut s = specified();
    let schema = BoardSetup::schema();
    let properties = &schema["properties"]["stackup"]["properties"]["strata"]["items"]["oneOf"][0]["properties"];
    assert_eq!(properties["role"]["type"], "string");
    assert_eq!(properties["role"]["enum"], json!(["signal", "plane", "mixed"]));
    assert_eq!(properties["thickness_basis"]["type"], "string");
    assert_eq!(properties["thickness_basis"]["enum"], json!(["nominal", "minimum"]));
    for role in ["signal", "plane", "mixed"] {
        for basis in ["nominal", "minimum"] {
            let before = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
            let mut candidate = setup(&s);
            candidate["stackup"]["strata"][0]["role"] = json!(role);
            candidate["stackup"]["strata"][0]["thickness_basis"] = json!(basis);
            // Force a real edit even when the enum strings already equal the candidate.
            candidate["stackup"]["strata"][0]["thickness_nm"] = json!(s.revision() + 47_900);
            apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
            assert_eq!(setup(&s), candidate);
            let documents = saved_board(&s, "pcbcraft-board/23");
            let reopened = persistence::decode_documents(&documents).unwrap();
            assert_eq!(serde_json::to_value(reopened.board_setup().unwrap()).unwrap(), candidate);
            let profile =
                SetupProfile::new("Copper strings".into(), s.project().unwrap().board_setup().unwrap().clone(), Settings::default()).unwrap();
            let encoded = profile.encode().unwrap();
            let profile_json: Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(profile_json["format"], "pcbcraft-setup-profile/2");
            let reopened = SetupProfile::decode(&encoded).unwrap();
            assert_eq!(serde_json::to_value(reopened.setup).unwrap(), candidate);
            s.apply("edit.undo", &json!({})).unwrap();
            assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
            s.apply("edit.redo", &json!({})).unwrap();
            assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), documents);
        }
    }
}

/// Known-variant maps and other wrong types fail before adoption on all three input routes.
#[test]
fn copper_role_and_thickness_basis_maps_are_refused_on_all_paths() {
    let mut s = specified();
    let base = setup(&s);
    let documents = saved_board(&s, "pcbcraft-board/23");
    let board_path = persistence::test_board_path(&documents);
    let board: Value = serde_json::from_slice(&documents[&board_path]).unwrap();
    let profile = SetupProfile::new("Copper strings".into(), s.project().unwrap().board_setup().unwrap().clone(), Settings::default()).unwrap();
    let profile: Value = serde_json::from_slice(&profile.encode().unwrap()).unwrap();
    for (field, variants) in [("role", &["signal", "plane", "mixed"][..]), ("thickness_basis", &["nominal", "minimum"][..])] {
        let pointer = format!("/stackup/strata/0/{field}");
        let mut cases = vec![json!({}), json!([]), json!(true), json!(1), Value::Null, json!("other"), json!("")];
        for variant in variants {
            cases.push(json!({(*variant):null}));
            cases.push(json!({(*variant):[]}));
        }
        cases.push(json!({(variants[0]):null,(variants[1]):null}));
        for value in cases {
            let reason = if value.is_string() { "unknown variant" } else { "string" };
            let mut candidate = base.clone();
            *candidate.pointer_mut(&pointer).unwrap() = value.clone();
            let params = json!({"setup":candidate,"expected_revision":s.revision()});
            reject_because(&mut s, SET_BOARD_SETUP, params, reason);
            let mut changed = documents.clone();
            let mut malformed_board = board.clone();
            malformed_board["setup"] = candidate.clone();
            changed.insert(board_path.clone(), serde_json::to_vec(&malformed_board).unwrap());
            refused_because(&changed, reason);
            let mut malformed_profile = profile.clone();
            malformed_profile["setup"] = candidate;
            let error = SetupProfile::decode(&serde_json::to_vec(&malformed_profile).unwrap()).unwrap_err();
            assert!(error.contains(reason), "profile {field}={value}: expected {reason}, got {error}");
            assert_eq!(setup(&s), base);
            assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), documents);
        }
    }
}
