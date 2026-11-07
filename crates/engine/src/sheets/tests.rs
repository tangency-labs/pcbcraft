use super::*;
use crate::persistence::{DocumentsEncoder, decode_documents, decode_packed, encode, encode_documents, encode_packed};

/// Issue commands at the current revision, retaining their ordinary strict validation.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Geometry, references and history remain independent of whichever projection a reader holds.
#[test]
fn sheets_are_explicit_isolated_and_share_one_undo_history() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    let first = s.snapshot().unwrap();
    apply(&mut s, CREATE_SHEET, json!({"name":"Power"}));
    let power = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":power.id(),"x_nm":0,"y_nm":0}));
    let project = s.project().unwrap();
    let power_view = project.for_sheet(power).unwrap();
    assert_eq!(project.symbols().len(), 1);
    assert_eq!(power_view.symbols().len(), 1);
    assert_eq!(project.reference(&project.symbols()[0]), "R1");
    assert_eq!(power_view.reference(&power_view.symbols()[0]), "R2");
    assert_eq!(project.connectivity().unconnected.len(), 2);
    assert_eq!(power_view.connectivity().unconnected.len(), 2);
    let before = s.snapshot().unwrap();
    for params in [json!({"x_nm":0,"y_nm":0}), json!({"sheet":uuid::Uuid::nil(),"x_nm":0,"y_nm":0})] {
        let mut params = params;
        params["expected_revision"] = json!(s.revision());
        assert!(s.apply(PLACE_RESISTOR, &params).is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    // Even a globally valid object address cannot escape the explicitly chosen sheet.
    let symbol = first.symbols()[0].id();
    let revision = s.revision();
    assert!(s.apply(MOVE_SYMBOL, &json!({"sheet":power.id(),"symbol":symbol,"x_nm":100,"y_nm":0,"expected_revision":revision})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(s.project().unwrap().for_sheet(power).unwrap().symbols().is_empty());
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&first, &s.snapshot().unwrap()));
    assert!(s.project().unwrap().for_sheet(main).is_some());
    s.apply("edit.redo", &Value::Null).unwrap();
    s.apply("edit.redo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Names, whole-sheet deletion and board associations are one revision-checked transaction.
#[test]
fn sheet_lifecycle_is_atomic_and_undo_restores_placements_and_footprints() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let main = s.project().unwrap().sheet_target();
    let revision = s.revision();
    assert!(s.apply(DELETE_SHEET, &json!({"sheet":main.id(),"expected_revision":revision})).is_err());
    apply(&mut s, CREATE_SHEET, json!({"name":"Power"}));
    let target = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":target.id(),"x_nm":0,"y_nm":0}));
    let symbol = s.project().unwrap().for_sheet(target).unwrap().symbols()[0].id();
    apply(
        &mut s,
        PLACE_FOOTPRINT,
        json!({"sheet":target.id(),"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0}),
    );
    apply(&mut s, RENAME_SHEET, json!({"sheet":target.id(),"name":"Supply"}));
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    apply(&mut s, RENAME_SHEET, json!({"sheet":target.id(),"name":"Supply"}));
    assert_eq!(s.revision(), revision);
    for (command, params) in [
        (CREATE_SHEET, json!({"name":"sUPPLY"})),
        (RENAME_SHEET, json!({"sheet":target.id(),"name":"Main"})),
        (RENAME_SHEET, json!({"sheet":target.id(),"name":"\n"})),
    ] {
        let mut params = params;
        params["expected_revision"] = json!(s.revision());
        assert!(s.apply(command, &params).is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    apply(&mut s, DELETE_SHEET, json!({"sheet":target.id()}));
    assert!(s.project().unwrap().for_sheet(target).is_none());
    assert_eq!(s.project().unwrap().components().count(), 0);
    assert_eq!(s.project().unwrap().boards().next().unwrap().footprints().count(), 0);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let revision = s.revision();
    assert!(s.apply(DELETE_SHEET, &json!({"sheet":target.id(),"expected_revision":revision-1})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Split and packed codecs preserve document identities, order and isolated wire ownership;
/// incremental encoding agrees at every batch size and moving a part changes only its sheet.
#[test]
fn multi_sheet_storage_is_lossless_local_and_rejects_cross_sheet_links() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    apply(&mut s, CREATE_SHEET, json!({"name":"Power"}));
    let power = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":power.id(),"x_nm":0,"y_nm":0}));
    let power_symbol = s.project().unwrap().for_sheet(power).unwrap().symbols()[0].id();
    apply(&mut s, DRAW_WIRE, json!({"sheet":power.id(),"from":{"symbol":power_symbol,"terminal":"1"},"to":{"x_nm":-20_000_000,"y_nm":0},"bends":[]}));
    let snapshot = s.snapshot().unwrap();
    let documents = encode_documents(snapshot.clone()).unwrap();
    assert!(encode(snapshot.clone()).is_err());
    let restored = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(restored.clone()).unwrap(), documents);
    assert_eq!(restored.for_sheet(power).unwrap().wires().len(), 1);
    assert!(restored.for_sheet(main).unwrap().wires().is_empty());
    let packed = encode_packed(snapshot.clone()).unwrap();
    assert_eq!(encode_documents(decode_packed(&packed).unwrap().project).unwrap(), documents);
    for batch in [1, 2, 17, 256] {
        let mut encoder = DocumentsEncoder::new(snapshot.clone());
        loop {
            if let Some(encoded) = encoder.step(batch).unwrap() {
                assert_eq!(encoded, documents);
                break;
            }
        }
    }
    // A projection cannot accidentally produce a partial save.
    assert_eq!(encode_documents(Arc::new(snapshot.for_sheet(power).unwrap())).unwrap(), documents);
    apply(&mut s, MOVE_SYMBOL, json!({"sheet":power.id(),"symbol":power_symbol,"x_nm":0,"y_nm":1_000_000}));
    let moved = encode_documents(s.snapshot().unwrap()).unwrap();
    let changed: Vec<_> = documents.keys().filter(|path| documents.get(*path) != moved.get(*path)).collect();
    assert_eq!(changed, vec![&format!("sheets/{}.json", power.id())]);
    let path = format!("sheets/{}.json", power.id());
    let mut forged = documents.clone();
    let mut doc: Value = serde_json::from_slice(&forged[&path]).unwrap();
    doc["wires"][0]["from"] = json!({"symbol":snapshot.symbols()[0].ordinal,"pin":"1"});
    forged.insert(path, serde_json::to_vec(&doc).unwrap());
    assert!(decode_documents(&forged).is_err());
    let mut adopted = Session::default();
    adopted.open_snapshot(restored);
    assert!(adopted.revision() > snapshot.highest_id());
}

/// Ambiguous targets are documented in the MCP schema; UUID/null and unknown inputs stay strict.
#[test]
fn every_schematic_command_discovers_its_sheet_parameter() {
    for command in CATALOG.iter().filter(|c| scoped_command(c.id)) {
        assert_eq!(command.to_json()["params"]["properties"]["sheet"]["format"], "uuid");
    }
}
