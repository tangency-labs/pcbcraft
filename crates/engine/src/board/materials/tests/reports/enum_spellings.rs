//! Report enum fields have the same string-only shape on command, native and profile paths.

use super::*;
use crate::board::setup::profile::SetupProfile;

/// Each occurrence is tested, including shared axis and basis types in different reports.
const ENUM_FIELDS: &[(&str, &[&str])] = &[
    ("/thermal_conductivity/0/axis", &["x", "y", "z"]),
    ("/thermal_conductivity/0/basis", &["typical", "minimum", "maximum", "measured"]),
    ("/cte/0/axis", &["x", "y", "z"]),
    ("/cte/0/region", &["below_tg", "above_tg"]),
    ("/cte/0/basis", &["typical", "minimum", "maximum", "measured"]),
    ("/dielectric_strength/0/axis", &["x", "y", "z"]),
    ("/dielectric_strength/0/basis", &["typical", "minimum", "maximum", "measured"]),
    ("/cti/0/material_group", &["I", "II", "IIIa", "IIIb"]),
    ("/cti/0/basis", &["typical", "minimum", "maximum", "measured"]),
];

/// Every report enum field accepts each named string and refuses its known-variant singleton
/// map on all three paths. Valid command-built fixtures establish the prerequisites before
/// type refusals; commands use the current revision and leave snapshot, history and bytes alone.
#[test]
fn report_enum_strings_refuse_singleton_maps_on_every_path() {
    let schema = crate::board::materials::schema::catalog();
    for &(path, variants) in ENUM_FIELDS {
        let parts: Vec<_> = path.split('/').collect();
        let scalar = &schema["items"]["properties"][parts[1]]["items"]["properties"][parts[3]];
        assert_eq!(scalar["enum"], json!(variants), "schema {path}");
        let mut s = session();
        let original = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        let mut base = reported(&s);
        // Keep group representation independent of the voltage/group consistency check.
        base["materials"][0]["cti"][0].as_object_mut().unwrap().remove("cti_v").unwrap();
        for &variant in variants {
            let mut candidate = base.clone();
            *candidate["materials"][0].pointer_mut(path).unwrap() = json!(variant);
            apply(&mut s, SET_BOARD_SETUP, json!({"setup":candidate}));
            let canonical = setup(&s);
            assert_eq!(canonical["materials"][0].pointer(path).unwrap(), &json!(variant), "{path}");
            assert_eq!(canonical["materials"][0]["tg"], base["materials"][0]["tg"]);
            assert_eq!(canonical["materials"][0]["dielectric"], base["materials"][0]["dielectric"]);
            let documents = saved(&s, "pcbcraft-board/24");
            let board_path = persistence::test_board_path(&documents);
            let profile = SetupProfile::new("Synthetic report enums".into(), s.project().unwrap().board_setup().unwrap().clone(), Default::default())
                .unwrap()
                .encode()
                .unwrap();
            assert_eq!(SetupProfile::decode(&profile).unwrap().encode().unwrap(), profile);
            let mut board = board_json(&documents);
            let mut profile_json: Value = serde_json::from_slice(&profile).unwrap();
            s.apply("edit.undo", &json!({})).unwrap();
            assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), original);

            let map = json!({(variant):null});
            let reason = "invalid type: map, expected a string";
            let mut rejected = canonical;
            *rejected["materials"][0].pointer_mut(path).unwrap() = map.clone();
            refuse(&mut s, rejected, reason);

            let pointer = format!("/setup/materials/0{path}");
            *board.pointer_mut(&pointer).unwrap() = map.clone();
            let mut changed = documents.clone();
            changed.insert(board_path, serde_json::to_vec(&board).unwrap());
            let error = persistence::decode_documents(&changed).map(|_| ()).unwrap_err();
            assert!(error.contains(reason), "native {path} = {variant:?} map: {error}");
            *profile_json.pointer_mut(&pointer).unwrap() = map;
            let error = SetupProfile::decode(&serde_json::to_vec(&profile_json).unwrap()).unwrap_err();
            assert!(error.contains(reason), "profile {path} = {variant:?} map: {error}");
            assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), original);
        }
    }
}
