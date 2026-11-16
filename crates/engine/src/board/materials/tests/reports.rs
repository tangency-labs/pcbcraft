//! Thermal conductivity, thermal expansion, dielectric strength and tracking reports.
use super::*;
use crate::board::materials::{MaterialGroup, MeasurementAxis, TgRegion};

/// Saved board documents, after checking the header and a lossless reopen.
fn saved(s: &Session, format: &str) -> persistence::Documents {
    let documents = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let board: Value = serde_json::from_slice(&documents[&persistence::test_board_path(&documents)]).unwrap();
    assert_eq!(board["format"], format);
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
    documents
}

/// The saved board document as JSON.
fn board_json(documents: &persistence::Documents) -> Value {
    serde_json::from_slice(&documents[&persistence::test_board_path(documents)]).unwrap()
}

/// Replace the board header of saved documents, keeping its body.
fn with_header(documents: &persistence::Documents, from: &str, to: &str) -> persistence::Documents {
    let path = persistence::test_board_path(documents);
    let text = String::from_utf8(documents[&path].clone()).unwrap();
    assert!(text.contains(from));
    let mut changed = documents.clone();
    changed.insert(path, text.replace(from, to).into_bytes());
    changed
}

/// One report of each kind, as the done test states them, each with its own source.
fn reports() -> Value {
    let source = |document: &str| json!({"document":document,"revision":"B","locator":"synthetic table 2"});
    json!({
        "thermal_conductivity":[{"conductivity_mw_per_m_k":1000,"axis":"z","temperature_milli_c":25000,
            "method":"Synthetic laser flash","basis":"typical","source":source("Synthetic thermal report")}],
        "cte":[{"cte_ppb_per_k":50000,"axis":"z","region":"below_tg","from_milli_c":50000,"to_milli_c":110000,
            "method":"Synthetic TMA","basis":"typical","source":source("Synthetic expansion report")}],
        "dielectric_strength":[{"breakdown_v":40000,"specimen_thickness_nm":1000000,"axis":"z",
            "method":"Synthetic short-time test","basis":"minimum","source":source("Synthetic breakdown report")}],
        "cti":[{"cti_v":175,"material_group":"IIIa","method":"Synthetic solution A test","basis":"minimum",
            "source":source("Synthetic tracking report")}]
    })
}

/// A candidate whose first material carries all four report kinds beside its Tg and Dk/Df reports.
fn reported(s: &Session) -> Value {
    let mut c = candidate(s);
    for (field, value) in reports().as_object().unwrap() {
        c["materials"][0][field] = value.clone();
    }
    c
}

/// Adding thermal conductivity (1.0 W/(m·K), z), thermal expansion (50 ppm/K, z, below Tg),
/// dielectric strength (40 kV at 1.0 mm) and CTI (175 V, group IIIa), each with a source, to a
/// new revision of a pinned material is one undoable edit that reads back with its unit, axis and
/// source and saves losslessly in board format 24. The Tg and Dk/Df reports and the earlier pins
/// stay unchanged, older headers refuse the new reports, and undo restores the format-3 bytes.
///
/// 175 V lies in group IIIa because IEC 60664-1:2007 clause 4.8.1.3 places IIIa from 175 V up to
/// below 400 V. The other values are synthetic.
#[test]
fn thermal_and_insulation_reports_round_trip_in_board_format_24() {
    let mut s = session();
    let c = candidate(&s);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let before = setup(&s);
    let plain = saved(&s, "pcbcraft-board/3");
    let snapshot = s.snapshot().unwrap();
    let mut c = before.clone();
    let mut revised = before["materials"][0].clone();
    revised["revision"] = json!(2);
    for (field, value) in reports().as_object().unwrap() {
        revised[field] = value.clone();
    }
    c["materials"].as_array_mut().unwrap().push(revised);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));

    let inspected = setup(&s);
    assert_eq!(inspected["materials"][0], before["materials"][0]);
    assert_eq!(inspected["materials"][2], before["materials"][1]);
    let material = &inspected["materials"][1];
    assert_eq!(material["revision"], 2);
    let conductivity = &material["thermal_conductivity"][0];
    assert_eq!(conductivity["conductivity_mw_per_m_k"], 1000);
    assert_eq!(conductivity["axis"], "z");
    assert_eq!(conductivity["source"]["document"], "Synthetic thermal report");
    let cte = &material["cte"][0];
    assert_eq!((cte["cte_ppb_per_k"].clone(), cte["axis"].clone(), cte["region"].clone()), (json!(50000), json!("z"), json!("below_tg")));
    assert_eq!(cte["source"]["document"], "Synthetic expansion report");
    let strength = &material["dielectric_strength"][0];
    assert_eq!((strength["breakdown_v"].clone(), strength["specimen_thickness_nm"].clone()), (json!(40000), json!(1000000)));
    assert_eq!(strength["source"]["document"], "Synthetic breakdown report");
    let cti = &material["cti"][0];
    assert_eq!((cti["cti_v"].clone(), cti["material_group"].clone()), (json!(175), json!("IIIa")));
    assert_eq!(cti["source"]["document"], "Synthetic tracking report");
    for field in ["tg", "dielectric"] {
        assert_eq!(material[field], before["materials"][0][field], "{field}");
    }
    assert!(inspected["materials"][0].get("thermal_conductivity").is_none());
    assert!(inspected["materials"][2].get("cti").is_none());

    let project = s.snapshot().unwrap();
    let materials = project.board_setup().unwrap().materials();
    let typed = &materials[1];
    assert_eq!(typed.thermal_conductivity[0].conductivity_mw_per_m_k, 1000);
    assert_eq!(typed.thermal_conductivity[0].axis, Some(MeasurementAxis::Z));
    assert_eq!(typed.cte[0].region, Some(TgRegion::BelowTg));
    assert_eq!(typed.dielectric_strength[0].specimen_thickness_nm, 1_000_000);
    assert_eq!(typed.cti[0].material_group, MaterialGroup::for_cti(175));
    assert_eq!(typed.tg, snapshot.board_setup().unwrap().materials()[0].tg);
    assert_eq!(typed.dielectric, snapshot.board_setup().unwrap().materials()[0].dielectric);

    let documents = saved(&s, "pcbcraft-board/24");
    let mut board = board_json(&documents);
    let old = board_json(&plain);
    let saved_materials = board["setup"]["materials"].as_array_mut().unwrap();
    let mut stripped = saved_materials.remove(1);
    for field in ["thermal_conductivity", "cte", "dielectric_strength", "cti"] {
        assert!(stripped.as_object_mut().unwrap().remove(field).is_some(), "{field}");
    }
    stripped["revision"] = json!(1);
    assert_eq!(stripped, old["setup"]["materials"][0]);
    assert_eq!(board["setup"], old["setup"]);
    let reopened = persistence::decode_documents(&documents).unwrap();
    assert_eq!(serde_json::to_value(reopened.board_setup().unwrap()).unwrap(), inspected);
    // Format 19 already fails on its own rules requirement; the others fail on the reports.
    for older in ["pcbcraft-board/3", "pcbcraft-board/19", "pcbcraft-board/20"] {
        let error = persistence::decode_documents(&with_header(&documents, "pcbcraft-board/24", older)).unwrap_err();
        assert!(older.ends_with("/19") || error.contains("format 24"), "{older}: {error}");
    }
    assert!(persistence::decode_documents(&with_header(&plain, "pcbcraft-board/3", "pcbcraft-board/24")).is_err());

    s.apply("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &snapshot));
    assert_eq!(saved(&s, "pcbcraft-board/3"), plain);
}

/// A report without its source, or with a bare value or a separate unit instead of the
/// unit-named field, is refused before adoption.
#[test]
fn reports_without_source_or_unit_are_refused() {
    let mut s = session();
    let base = reported(&s);
    let revision = s.revision();
    let values =
        [("thermal_conductivity", "conductivity_mw_per_m_k"), ("cte", "cte_ppb_per_k"), ("dielectric_strength", "breakdown_v"), ("cti", "cti_v")];
    for (kind, value) in values {
        for source in [None, Some(Value::Null), Some(json!({})), Some(json!({"document":""}))] {
            let mut c = base.clone();
            let report = c["materials"][0][kind][0].as_object_mut().unwrap();
            match source {
                Some(source) => report.insert("source".into(), source),
                None => report.remove("source"),
            };
            reject(&mut s, c, revision);
        }
        let mut c = base.clone();
        let report = c["materials"][0][kind][0].as_object_mut().unwrap();
        let number = report.remove(value).unwrap();
        report.insert("value".into(), number);
        reject(&mut s, c.clone(), revision);
        c["materials"][0][kind][0]["unit"] = json!("SI");
        reject(&mut s, c, revision);
        let mut c = base.clone();
        c["materials"][0][kind][0]["unit"] = json!("SI");
        reject(&mut s, c, revision);
        if kind != "cti" {
            let mut c = base.clone();
            c["materials"][0][kind][0].as_object_mut().unwrap().remove(value);
            reject(&mut s, c, revision);
        }
    }
    let mut c = base.clone();
    c["materials"][0]["cti"][0].as_object_mut().unwrap().remove("cti_v");
    c["materials"][0]["cti"][0].as_object_mut().unwrap().remove("material_group");
    reject(&mut s, c, revision);
    let mut c = base;
    c["materials"][0]["dielectric_strength"][0].as_object_mut().unwrap().remove("specimen_thickness_nm");
    reject(&mut s, c, revision);
    assert_eq!(s.revision(), revision);
}

/// Each bound accepts its last good value and refuses the first bad one; wrong types, nulls,
/// unknown enum values, unknown fields, duplicates and contradicting CTI groups are refused.
#[test]
fn report_bounds_and_contradictions() {
    let mut s = session();
    let base = reported(&s);
    let accepted: &[(&str, Value)] = &[
        ("/thermal_conductivity/0/conductivity_mw_per_m_k", json!(1)),
        ("/thermal_conductivity/0/conductivity_mw_per_m_k", json!(10_000_000)),
        ("/thermal_conductivity/0/temperature_milli_c", json!(-273_149)),
        ("/cte/0/cte_ppb_per_k", json!(-1_000_000)),
        ("/cte/0/cte_ppb_per_k", json!(0)),
        ("/cte/0/cte_ppb_per_k", json!(10_000_000)),
        ("/cte/0/to_milli_c", json!(1_000_000)),
        ("/cte/0/region", json!("above_tg")),
        ("/dielectric_strength/0/breakdown_v", json!(1)),
        ("/dielectric_strength/0/breakdown_v", json!(10_000_000)),
        ("/dielectric_strength/0/specimen_thickness_nm", json!(1)),
        ("/dielectric_strength/0/specimen_thickness_nm", json!(100_000_000)),
        ("/cti/0", json!({"cti_v":1,"basis":"typical","source":{"document":"Synthetic"}})),
        ("/cti/0", json!({"cti_v":1000,"material_group":"I","basis":"typical","source":{"document":"Synthetic"}})),
        ("/cti/0", json!({"material_group":"II","basis":"typical","source":{"document":"Synthetic"}})),
    ];
    for (path, value) in accepted {
        let mut c = base.clone();
        *c["materials"][0].pointer_mut(path).unwrap() = value.clone();
        apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
        assert_eq!(setup(&s)["materials"][0].pointer(path).unwrap(), value, "{path}");
        s.apply("edit.undo", &json!({})).unwrap();
    }
    for (cti, group) in [(100, "IIIb"), (174, "IIIb"), (175, "IIIa"), (399, "IIIa"), (400, "II"), (599, "II"), (600, "I"), (1000, "I")] {
        assert_eq!(MaterialGroup::for_cti(cti).map(MaterialGroup::name), Some(group));
        let mut c = base.clone();
        c["materials"][0]["cti"][0]["cti_v"] = json!(cti);
        c["materials"][0]["cti"][0]["material_group"] = json!(group);
        apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
        s.apply("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(MaterialGroup::for_cti(99), None);
    let revision_after = s.revision();
    let refused: &[(&str, Vec<Value>)] = &[
        ("/thermal_conductivity", vec![Value::Null, json!({})]),
        ("/thermal_conductivity/0/conductivity_mw_per_m_k", vec![json!(0), json!(10_000_001), json!(1.5), json!("1 W/(m K)"), Value::Null]),
        ("/thermal_conductivity/0/axis", vec![json!("isotropic"), json!("through-plane"), Value::Null]),
        ("/thermal_conductivity/0/temperature_milli_c", vec![json!(-273_150), json!(1_000_001), Value::Null]),
        ("/thermal_conductivity/0/method", vec![json!(" "), json!("x".repeat(81)), Value::Null]),
        ("/thermal_conductivity/0/basis", vec![json!("guaranteed"), Value::Null]),
        ("/cte", vec![Value::Null]),
        ("/cte/0/cte_ppb_per_k", vec![json!(-1_000_001), json!(10_000_001), json!(50.5), Value::Null]),
        ("/cte/0/region", vec![json!("at_tg"), json!("below"), Value::Null]),
        ("/cte/0/from_milli_c", vec![json!(110_000), json!(120_000), json!(-273_150), Value::Null]),
        ("/cte/0/to_milli_c", vec![json!(50_000), json!(1_000_001), Value::Null]),
        ("/dielectric_strength", vec![Value::Null]),
        ("/dielectric_strength/0/breakdown_v", vec![json!(0), json!(10_000_001), json!(-1), json!("40 kV"), Value::Null]),
        ("/dielectric_strength/0/specimen_thickness_nm", vec![json!(0), json!(100_000_001), Value::Null]),
        ("/dielectric_strength/0/axis", vec![json!("parallel")]),
        ("/cti", vec![Value::Null]),
        ("/cti/0/cti_v", vec![json!(0), json!(1001), json!(174), json!(400), Value::Null]),
        ("/cti/0/material_group", vec![json!("IV"), json!("iiia"), json!("3a"), json!("IIIb"), Value::Null]),
        ("/cti/0/method", vec![json!("")]),
    ];
    for (path, values) in refused {
        for value in values {
            let mut c = base.clone();
            *c["materials"][0].pointer_mut(path).unwrap() = value.clone();
            reject(&mut s, c, revision_after);
        }
    }
    for (kind, remove) in [("cte", "from_milli_c"), ("cte", "to_milli_c")] {
        let mut c = base.clone();
        c["materials"][0][kind][0].as_object_mut().unwrap().remove(remove);
        reject(&mut s, c, revision_after);
    }
    let mut c = base.clone();
    c["materials"][0]["cti"][0] = json!({"cti_v":99,"material_group":"IIIb","basis":"typical","source":{"document":"Synthetic"}});
    reject(&mut s, c, revision_after);
    for kind in ["thermal_conductivity", "cte", "dielectric_strength", "cti"] {
        let mut c = base.clone();
        c["materials"][0][kind][0]["future_field"] = json!(true);
        reject(&mut s, c, revision_after);
        let mut c = base.clone();
        c["materials"][0][kind][0]["source"]["future_field"] = json!(true);
        reject(&mut s, c, revision_after);
        let mut c = base.clone();
        let report = c["materials"][0][kind][0].clone();
        c["materials"][0][kind].as_array_mut().unwrap().push(report);
        reject(&mut s, c, revision_after);
        for count in [16, 17] {
            let mut c = base.clone();
            let report = c["materials"][0][kind][0].clone();
            c["materials"][0][kind] = json!(
                (0..count)
                    .map(|n| {
                        let mut r = report.clone();
                        r["source"]["document"] = json!(format!("Synthetic {n}"));
                        r
                    })
                    .collect::<Vec<_>>()
            );
            if count == 16 {
                apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
                assert_eq!(setup(&s)["materials"][0][kind].as_array().unwrap().len(), 16);
                s.apply("edit.undo", &json!({})).unwrap();
            } else {
                reject(&mut s, c, revision_after);
            }
        }
    }
}

/// Report order is not authored: a reordered candidate is a no-op, and missing optional
/// conditions stay absent instead of becoming defaults.
#[test]
fn report_order_is_canonical_and_missing_conditions_stay_unknown() {
    let mut s = session();
    let mut c = reported(&s);
    c["materials"][0]["cte"] = json!([
        {"cte_ppb_per_k":250000,"basis":"typical","source":{"document":"Synthetic"}},
        {"cte_ppb_per_k":-500,"axis":"x","basis":"typical","source":{"document":"Synthetic"}}
    ]);
    c["materials"][0]["cti"] = json!([{"material_group":"IIIb","basis":"minimum","source":{"document":"Synthetic"}}]);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let revision = s.revision();
    let actual = setup(&s);
    assert_eq!(actual["materials"][0]["cte"][0]["cte_ppb_per_k"], -500);
    let open = &actual["materials"][0]["cte"][1];
    for field in ["axis", "region", "from_milli_c", "to_milli_c", "method"] {
        assert!(open.get(field).is_none(), "{field}");
    }
    assert!(actual["materials"][0]["cti"][0].get("cti_v").is_none());
    let mut reordered = actual.clone();
    reordered["materials"][0]["cte"].as_array_mut().unwrap().reverse();
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":reordered}));
    assert_eq!(s.revision(), revision);
}

/// Format 24 also carries format 20's stack fields and format 19's rules; dropping the reports
/// returns to the format the remaining data needs.
#[test]
fn board_format_24_carries_stack_fields_and_rules() {
    let mut s = crate::board::track::tests::routed_fixture();
    let board = s.project().unwrap().board_target().unwrap().board;
    apply(&mut s, crate::board::rules::SET_RULES, json!({"board":board,"rules":crate::board::rules::Values::default()}));
    let ruled = saved(&s, "pcbcraft-board/19");
    let mut c = setup(&s);
    c["stackup"]["strata"][0]["role"] = json!("signal");
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c.clone()}));
    let stacked = saved(&s, "pcbcraft-board/20");
    let mut material = json!({"id":Uuid::from_u128(301),"revision":1,"name":"Synthetic insulation"});
    for (field, value) in reports().as_object().unwrap() {
        material[field] = value.clone();
    }
    c["materials"] = json!([material]);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let documents = saved(&s, "pcbcraft-board/24");
    assert!(board_json(&documents)["rules"].is_object());
    assert_eq!(setup(&s)["stackup"]["strata"][0]["role"], "signal");
    assert!(persistence::decode_documents(&with_header(&documents, "pcbcraft-board/24", "pcbcraft-board/20")).is_err());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(saved(&s, "pcbcraft-board/20"), stacked);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(saved(&s, "pcbcraft-board/19"), ruled);
}

/// Locked copper also travels in format 24 without any format 20 stack field, and dropping the
/// reports returns to format 18 byte for byte.
#[test]
fn board_format_24_carries_locked_copper() {
    use crate::board::track::{ADD_VIA, SET_COPPER_LOCKED};
    let mut s = crate::board::track::tests::routed_fixture();
    let board = s.project().unwrap().board_target().unwrap().board;
    apply(&mut s, ADD_VIA, json!({"board":board,"at":{"x_nm":0,"y_nm":40_000_000}}));
    let via = s.project().unwrap().board.vias[0].id;
    apply(&mut s, SET_COPPER_LOCKED, json!({"board":board,"vias":[via],"locked":true}));
    let locked = saved(&s, "pcbcraft-board/18");
    let mut c = setup(&s);
    let mut material = json!({"id":Uuid::from_u128(302),"revision":1,"name":"Synthetic insulation"});
    material["cti"] = reports()["cti"].clone();
    c["materials"] = json!([material]);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let documents = saved(&s, "pcbcraft-board/24");
    let reopened = persistence::decode_documents(&documents).unwrap();
    assert!(reopened.board.vias.iter().any(|via| via.locked));
    assert!(persistence::decode_documents(&with_header(&documents, "pcbcraft-board/24", "pcbcraft-board/18")).is_err());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(saved(&s, "pcbcraft-board/18"), locked);
}
