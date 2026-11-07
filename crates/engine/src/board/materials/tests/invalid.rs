//! Refusal paths use candidates and malformed derivatives of production-encoded documents.
use super::*;

/// Unknown semantics, invalid measurements, aliases and nulls never partially adopt a setup.
#[test]
fn invalid_materials_and_construction_preserve_state() {
    let mut s = session();
    s.apply("project.rename", &json!({"name":"Keep redo"})).unwrap();
    s.apply("edit.undo", &json!({})).unwrap();
    let base = candidate(&s);
    let revision = s.revision();
    for (path, values) in [
        ("/materials", vec![Value::Null]),
        ("/materials/0/id", vec![json!(Uuid::nil()), base["materials"][1]["id"].clone()]),
        ("/materials/0/revision", vec![json!(0), json!(-1), json!(1.5), json!(u64::MAX)]),
        ("/materials/0/name", vec![json!(" "), json!(" padded"), json!("a\nb"), json!("x".repeat(81))]),
        ("/materials/0/source/document", vec![json!(""), Value::Null]),
        ("/materials/0/tg", vec![Value::Null]),
        ("/materials/0/tg/0/temperature_milli_c", vec![json!(-273150), json!(1000001), json!(1.25), json!("160 C"), Value::Null]),
        ("/materials/0/tg/0/method", vec![json!("future_method")]),
        ("/materials/0/tg/0/basis", vec![json!("required_minimum")]),
        ("/materials/0/tg/0/source", vec![json!({}), Value::Null]),
        ("/materials/0/dielectric", vec![Value::Null]),
        ("/materials/0/dielectric/0/dk_millionths", vec![json!(999999), json!(1000000001), json!(4.2), Value::Null]),
        ("/materials/0/dielectric/0/df_millionths", vec![json!(-1), json!(1000001), Value::Null]),
        ("/materials/0/dielectric/0/frequency_hz", vec![json!(0), json!(1000000000000001_u64), json!(1.5), Value::Null]),
        ("/materials/0/dielectric/0/temperature_milli_c", vec![json!(-273150), json!(1000001)]),
        ("/materials/0/dielectric/0/method", vec![json!(" ")]),
        ("/materials/0/dielectric/0/axis", vec![json!("isotropic")]),
        ("/materials/0/dielectric/0/value_kind", vec![json!("universal")]),
        ("/materials/0/dielectric/0/resin_content_ppm", vec![json!(0), json!(1000000)]),
        ("/materials/0/dielectric/0/specimen_thickness_nm", vec![json!(0), json!(100000001)]),
        ("/stackup/strata/1/construction", vec![Value::Null]),
        ("/stackup/strata/1/construction/kind", vec![json!("future_bonding")]),
        ("/stackup/strata/1/construction/plies", vec![json!([]), Value::Null]),
        (
            "/stackup/strata/1/construction/plies/0/id",
            vec![json!(Uuid::nil()), base["stackup"]["id"].clone(), base["layers"][0]["id"].clone(), json!(Uuid::from_u128(203))],
        ),
        ("/stackup/strata/1/construction/plies/0/material/revision", vec![json!(0), json!(2)]),
        ("/stackup/strata/1/construction/plies/0/resin_content_ppm", vec![json!(0), json!(1000000), json!(55.5)]),
        ("/stackup/strata/1/construction/plies/0/nominal_thickness_nm", vec![json!(0), json!(-1), json!(100000001), Value::Null]),
        ("/stackup/strata/1/thickness_tolerance_nm", vec![json!(-1), json!(200000), json!(i64::MAX), Value::Null]),
    ] {
        for value in values {
            let mut c = base.clone();
            *c.pointer_mut(path).unwrap() = value;
            reject(&mut s, c, revision);
        }
    }
    for path in [
        "/materials/0",
        "/materials/0/tg/0",
        "/materials/0/source",
        "/materials/0/dielectric/0",
        "/stackup/strata/3/construction",
        "/stackup/strata/1/construction/plies/0",
    ] {
        let mut c = base.clone();
        c.pointer_mut(path).unwrap()["future_field"] = json!(true);
        reject(&mut s, c, revision);
    }
    for field in ["tg", "dielectric"] {
        let mut c = base.clone();
        let report = c["materials"][0][field][0].clone();
        c["materials"][0][field].as_array_mut().unwrap().push(report);
        reject(&mut s, c, revision);
    }
    let mut c = base.clone();
    c["materials"][0]["dielectric"][0].as_object_mut().unwrap().remove("dk_millionths");
    c["materials"][0]["dielectric"][0].as_object_mut().unwrap().remove("df_millionths");
    reject(&mut s, c, revision);
    let mut c = base;
    c["stackup"]["strata"][1].as_object_mut().unwrap().remove("thickness_nm");
    reject(&mut s, c, revision);
    assert!(s.available("edit.redo"));
}

/// Large catalogs and construction recipes are bounded before snapshot adoption.
#[test]
fn collection_bounds_are_enforced() {
    let mut s = session();
    let base = candidate(&s);
    let revision = s.revision();
    let mut c = base.clone();
    c["materials"] = json!((1..=65).map(|n| json!({"id":Uuid::from_u128(100+n),"revision":1,"name":"Synthetic"})).collect::<Vec<_>>());
    reject(&mut s, c, revision);
    for (field, count) in [("tg", 9), ("dielectric", 33)] {
        let mut c = base.clone();
        let report = c["materials"][0][field][0].clone();
        c["materials"][0][field] = json!(
            (0..count)
                .map(|n| {
                    let mut r = report.clone();
                    r["source"]["document"] = json!(format!("Synthetic {n}"));
                    r
                })
                .collect::<Vec<_>>()
        );
        reject(&mut s, c, revision);
    }
    let mut c = base;
    c["stackup"]["strata"][1]["construction"]["plies"] = json!((0..33).map(|n| json!({"id":Uuid::from_u128(1000+n)})).collect::<Vec<_>>());
    reject(&mut s, c, revision);
}

/// Several independently valid ply groups still obey the whole-board resource limit.
#[test]
fn total_ply_limit_applies_across_gaps() {
    let mut s = session();
    for total in [256_usize, 257] {
        let mut c = setup(&s);
        let top = c["stackup"]["strata"][0].clone();
        let bottom = c["stackup"]["strata"].as_array().unwrap().last().unwrap().clone();
        c["layers"].as_array_mut().unwrap().retain(|l| l["purpose"] != "copper" || l["id"] == top["layer"] || l["id"] == bottom["layer"]);
        let mut strata = vec![top];
        for (gap, start) in (0..total).step_by(32).enumerate() {
            if gap != 0 {
                let layer = json!({"id":Uuid::from_u128(3000+gap as u128),"name":format!("Inner {gap}"),"purpose":"copper"});
                strata.push(json!({"kind":"copper","layer":layer["id"]}));
                c["layers"].as_array_mut().unwrap().push(layer);
            }
            let plies: Vec<_> = (start..(start + 32).min(total)).map(|i| json!({"id":Uuid::from_u128(4000+i as u128)})).collect();
            strata.push(json!({"kind":"dielectric","id":Uuid::from_u128(5000+gap as u128),"construction":{"kind":"prepreg","plies":plies}}));
        }
        strata.push(bottom);
        c["stackup"]["strata"] = json!(strata);
        if total == 256 {
            apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
            assert!(s.snapshot().unwrap().board_setup().unwrap().laminate_thickness_nm().is_none());
        } else {
            let revision = s.revision();
            reject(&mut s, c, revision);
        }
    }
}

/// Missing characterization remains unknown, whereas zero loss and zero Celsius are real values.
#[test]
fn optional_report_conditions_and_zero_values_are_not_defaults() {
    let mut s = session();
    let mut c = candidate(&s);
    c["materials"][0]["tg"][0]["temperature_milli_c"] = json!(0);
    c["materials"][0]["dielectric"] = json!([{"df_millionths":0,"basis":"measured","source":{"document":"Synthetic"}}]);
    c["stackup"]["strata"][3]["construction"] = json!({"kind":"core"});
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let actual = setup(&s);
    let report = &actual["materials"][0]["dielectric"][0];
    assert_eq!(report["df_millionths"], 0);
    assert!(report.get("dk_millionths").is_none());
    assert!(report.get("frequency_hz").is_none());
    assert!(report.get("method").is_none());
    assert!(actual["stackup"]["strata"][3]["construction"].get("material").is_none());
    assert_eq!(actual["materials"][0]["tg"][0]["temperature_milli_c"], 0);
}

/// Loading refuses new semantics under old versions and duplicate keys in original JSON.
#[test]
fn unsupported_or_ambiguous_saved_materials_fail_before_open() {
    let mut s = session();
    let legacy = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let c = candidate(&s);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let docs = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let board: Value = serde_json::from_slice(&docs["board.json"]).unwrap();
    for format in ["pcbcraft-board/1", "pcbcraft-board/2", "pcbcraft-board/7"] {
        let mut bad = docs.clone();
        let mut b = board.clone();
        b["format"] = json!(format);
        bad.insert("board.json".to_owned(), serde_json::to_vec(&b).unwrap());
        assert!(persistence::decode_documents(&bad).unwrap_err().contains("board.json"));
    }
    let mut b: Value = serde_json::from_slice(&legacy["board.json"]).unwrap();
    b["setup"]["materials"] = json!([]);
    let mut bad = legacy;
    bad.insert("board.json".to_owned(), serde_json::to_vec(&b).unwrap());
    assert!(persistence::decode_documents(&bad).unwrap_err().contains("version 2"));
    let encoded = std::str::from_utf8(&docs["board.json"]).unwrap();
    for (needle, replacement) in
        [("\"family\":", "\"family\":\"Alias\",\"family\":"), ("\"dk_millionths\":", "\"dk_millionths\":2000000,\"dk_millionths\":")]
    {
        let mut bad = docs.clone();
        assert!(encoded.contains(needle));
        bad.insert("board.json".to_owned(), encoded.replacen(needle, replacement, 1).into_bytes());
        assert!(persistence::decode_documents(&bad).unwrap_err().contains("duplicate field"));
    }
}
