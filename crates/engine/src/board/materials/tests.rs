//! Acceptance through shared commands and codecs; example materials are synthetic, not product data.

use crate::{INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_BOARD_SETUP, Session, persistence};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

mod invalid;

/// Supply a fresh revision through the public command path.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}

/// Initialize the ordinary four-layer preset; no project documents are hand-authored.
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    apply(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":4}));
    s
}

/// Capture the public, editable setup representation.
fn setup(s: &Session) -> Value {
    s.inspect()["project"]["board"]["setup"].clone()
}

/// Original synthetic reports deliberately differ in method, frequency and material family.
fn candidate(s: &Session) -> Value {
    let mut c = setup(s);
    let first = json!({"id":Uuid::from_u128(101),"revision":1});
    let second = json!({"id":Uuid::from_u128(102),"revision":1});
    let source = json!({"document":"Synthetic characterization","revision":"A","locator":"local test record"});
    c["materials"] = json!([
        {"id":first["id"],"revision":1,"name":"Custom core","family":"User-defined ceramic composite","source":source,
         "tg":[
             {"temperature_milli_c":160125,"method":"dsc","basis":"typical","source":source},
             {"temperature_milli_c":170250,"method":"dma","basis":"measured","source":source}
         ],
         "dielectric":[
             {"dk_millionths":4200000,"df_millionths":15000,"frequency_hz":1000000000,"method":"Synthetic stripline test",
              "value_kind":"test","basis":"typical","source":source,"temperature_milli_c":23125,"axis":"z",
              "glass_construction":"Synthetic weave A","resin_content_ppm":550000,"specimen_thickness_nm":1000000},
             {"dk_millionths":4100000,"frequency_hz":5000000000_u64,"method":"Synthetic design fit",
              "value_kind":"design","basis":"typical","source":source}
         ]},
        {"id":second["id"],"revision":1,"name":"User bond material","family":"FR-4"}
    ]);
    let strata = c["stackup"]["strata"].as_array_mut().unwrap();
    for (i, stratum) in strata.iter_mut().enumerate() {
        stratum["thickness_nm"] = json!(if i % 2 == 0 {
            35000
        } else if i == 3 {
            1000000
        } else {
            200000
        });
    }
    strata[1]["thickness_tolerance_nm"] = json!(10000);
    strata[1]["construction"] = json!({"kind":"prepreg","plies":[
        {"id":Uuid::from_u128(201),"material":second,"glass_style":"Synthetic A","resin_content_ppm":550000,"nominal_thickness_nm":180000},
        {"id":Uuid::from_u128(202),"material":first,"nominal_thickness_nm":175000}
    ]});
    strata[3]["construction"] = json!({"kind":"core","material":first,"glass_construction":"2 x synthetic A"});
    strata[5]["construction"] = json!({"kind":"prepreg","plies":[{"id":Uuid::from_u128(203),"material":second,"nominal_thickness_nm":150000}]});
    c
}

/// Invalid or stale edits preserve the snapshot, histories, revision and production save bytes.
fn reject(s: &mut Session, c: Value, revision: u64) {
    let snapshot = s.snapshot().unwrap();
    let state = s.inspect();
    let saved = persistence::encode_documents(snapshot.clone()).unwrap();
    assert!(s.apply(SET_BOARD_SETUP, &json!({"setup":c,"expected_revision":revision})).is_err(), "accepted {c}");
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.inspect(), state);
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Mixed user materials, core and multiple prepreg plies survive edits on a populated board.
/// Electrical connectivity and finished thickness do not derive from nominal stock or reports.
#[test]
fn custom_materials_and_construction_are_one_undoable_edit() {
    let mut s = session();
    apply(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let symbol = s.inspect()["project"]["schematic"]["symbols"][0]["id"].clone();
    apply(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":25400000,"y_nm":0}));
    let other = s.inspect()["project"]["schematic"]["symbols"][1]["id"].clone();
    apply(&mut s, crate::CONNECT_PINS, json!({"from":{"symbol":symbol,"terminal":"2"},"to":{"symbol":other,"terminal":"1"}}));
    apply(&mut s, PLACE_FOOTPRINT, json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0}));
    let before = s.snapshot().unwrap();
    let netlist = s.inspect()["project"]["schematic"]["netlist"].clone();
    assert_eq!(netlist["nets"].as_array().unwrap().len(), 1);
    let c = candidate(&s);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let after = s.snapshot().unwrap();
    let inspected = setup(&s);
    assert_eq!(inspected["materials"][0]["family"], "User-defined ceramic composite");
    assert!(inspected["materials"][1].get("tg").is_none());
    assert!(inspected["materials"][1].get("dielectric").is_none());
    assert_eq!(inspected["materials"][0]["dielectric"].as_array().unwrap().len(), 2);
    assert_eq!(inspected["stackup"]["strata"], c["stackup"]["strata"]);
    assert_eq!(after.board_setup().unwrap().laminate_thickness_nm(), Some(1_540_000));
    assert_eq!(s.inspect()["project"]["schematic"]["netlist"], netlist);
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &before));
    s.apply("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &after));
}

/// Referenced revisions cannot be removed or rewritten; an update selects an explicit new pin.
#[test]
fn material_updates_are_explicit_and_stale_requests_are_atomic() {
    let mut s = session();
    let c = candidate(&s);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let revision = s.revision();
    let original = setup(&s);
    let mut changed = original.clone();
    changed["materials"][0]["dielectric"][0]["dk_millionths"] = json!(3900000);
    reject(&mut s, changed.clone(), revision);
    let mut removed = original.clone();
    removed["materials"].as_array_mut().unwrap().remove(0);
    reject(&mut s, removed, revision);
    let mut new_pin = changed["materials"][0].clone();
    new_pin["revision"] = json!(2);
    changed["materials"] = original["materials"].clone();
    changed["materials"].as_array_mut().unwrap().push(new_pin);
    changed["stackup"]["strata"][3]["construction"]["material"]["revision"] = json!(2);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":changed}));
    assert_eq!(setup(&s)["stackup"]["strata"][1]["construction"]["plies"][1]["material"]["revision"], 1);
    assert_eq!(setup(&s)["stackup"]["strata"][3]["construction"]["material"]["revision"], 2);
    reject(&mut s, original.clone(), revision);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(setup(&s), original);
    assert!(s.available("edit.redo"));
}

/// Catalog/report ordering is nonsemantic; ply order is authored and a gap edit has a local diff.
#[test]
fn no_op_catalog_reordering_preserves_redo_but_ply_order_is_authored() {
    let mut s = session();
    let c = candidate(&s);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    s.apply("project.rename", &json!({"name":"Later edit"})).unwrap();
    s.apply("edit.undo", &json!({})).unwrap();
    let revision = s.revision();
    let snapshot = s.snapshot().unwrap();
    let before = persistence::encode_documents(snapshot.clone()).unwrap();
    let mut c = setup(&s);
    c["materials"][0]["tg"].as_array_mut().unwrap().reverse();
    c["materials"][0]["dielectric"].as_array_mut().unwrap().reverse();
    c["materials"].as_array_mut().unwrap().reverse();
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &snapshot));
    assert!(s.available("edit.redo"));
    c["stackup"]["strata"][1]["construction"]["plies"].as_array_mut().unwrap().reverse();
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    assert_eq!(setup(&s)["stackup"]["strata"][1]["construction"]["plies"][0]["id"], json!(Uuid::from_u128(202)));
    assert!(!s.available("edit.redo"));
    let after = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let differing = std::str::from_utf8(&before["board.json"])
        .unwrap()
        .lines()
        .zip(std::str::from_utf8(&after["board.json"]).unwrap().lines())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(differing, 1);
}

/// Both codecs retain embedded reports offline, while an incremental encoder holds its snapshot
/// across a later edit. Version 2 remains byte-identical and never guesses construction.
#[test]
fn offline_codecs_preserve_materials_and_legacy_unknowns() {
    let mut s = session();
    let mut legacy = setup(&s);
    legacy["stackup"]["strata"][1]["material"] = json!("FR-4");
    legacy["stackup"]["strata"][1]["thickness_nm"] = json!(2540000);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":legacy}));
    let old = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&old["board.json"]).unwrap().contains("pcbcraft-board/2"));
    s.open_snapshot(persistence::decode_documents(&old).unwrap());
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), old);
    assert!(setup(&s).get("materials").is_none());
    assert!(setup(&s)["stackup"]["strata"][1].get("construction").is_none());
    let c = candidate(&s);
    apply(&mut s, SET_BOARD_SETUP, json!({"setup":c}));
    let snapshot = s.snapshot().unwrap();
    let docs = persistence::encode_documents(snapshot.clone()).unwrap();
    assert!(std::str::from_utf8(&docs["board.json"]).unwrap().contains("pcbcraft-board/3"));
    let packed = persistence::encode_packed(snapshot.clone()).unwrap();
    assert_eq!(persistence::encode_packed(persistence::decode_packed(&packed).unwrap().project).unwrap(), packed);
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&docs).unwrap()).unwrap(), docs);
    let mut encoder = persistence::DocumentsEncoder::new(snapshot);
    assert!(encoder.step(1).unwrap().is_none());
    s.apply("edit.undo", &json!({})).unwrap();
    let result = loop {
        if let Some(result) = encoder.step(1).unwrap() {
            break result;
        }
    };
    assert_eq!(result, docs);
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), old);
}
