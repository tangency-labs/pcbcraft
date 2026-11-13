//! Historical quantities use command-built native parts and production folder/archive codecs.
use super::*;
use crate::board::update::{APPLY_UPDATE, Baseline, INITIALIZE_BASELINE, PREVIEW_UPDATE};
use crate::library::parts::Quantity;

/// Derive an original two-terminal test part with an explicit primary quantity category.
fn typed_part(quantity: Quantity, input: Option<&str>) -> Session {
    let mut docs = documents();
    docs.remove("manufacturer-parts/test-resistor.json");
    let mut category: Json = serde_json::from_str(&docs["categories/resistor.json"]).unwrap();
    category["fields"] = json!({"parts-demo:primary":{"name":"Primary","value_type":{
        "type":"quantity","quantity":quantity,"unit":quantity.unit()}}});
    docs.insert("categories/resistor.json".into(), category.to_string());
    let mut part: Json = serde_json::from_str(&docs["parts/resistor.json"]).unwrap();
    part["category"]["primary_value"] = json!("parts-demo:primary");
    docs.insert("parts/resistor.json".into(), part.to_string());
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session.execute(sources::LOAD, &json!({"source":"native-parts","documents":docs})).unwrap();
    let params = placement(&session, false);
    apply(&mut session, PLACE_PART, params);
    if let Some(input) = input {
        set_value(&mut session, input);
    }
    let id = session.project().unwrap().components().next().unwrap().id();
    apply(&mut session, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":id,"x_nm":0,"y_nm":0}));
    session
}

/// Edit the fixture's primary value through the shared typed validation path.
fn set_value(session: &mut Session, input: &str) {
    let symbol = session.project().unwrap().symbols()[0].id();
    let reference = session.project().unwrap().components().next().unwrap().reference().to_owned();
    apply(session, crate::SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":reference,"value":input}));
}

/// Assert that the common board projection carries the expected historical display.
fn shown(project: &crate::Project, expected: &str, pending_removal: bool) {
    let footprint = project.boards().next().unwrap().footprints().next().unwrap();
    assert_eq!(footprint.value, expected);
    assert_eq!(footprint.pending_removal, pending_removal);
}

/// A real accepted update captures quantities; metadata alone never manufactures an update.
fn accept(session: &mut Session) {
    let report = session.execute(PREVIEW_UPDATE, &json!({"expected_revision":session.revision()})).unwrap();
    apply(session, APPLY_UPDATE, json!({"plan_token":report["board_update_preview"]["plan_token"]}));
}

/// Typed history survives every supported source-deletion path, codecs and undo/redo;
/// last-visible quantities follow edits made after the accepted capture.
#[test]
fn retained_primary_quantities_survive_deletion_history_and_reopen() {
    for mode in 0..3 {
        let mut session = typed_part(Quantity::Capacitance, Some("100n"));
        apply(&mut session, INITIALIZE_BASELINE, json!({}));
        let baseline = session.project().unwrap().board.baseline.as_ref().unwrap();
        assert_eq!(baseline.format, "pcbcraft-board-baseline/3");
        assert_eq!(
            baseline.components.values().next().unwrap().primary_quantity,
            Some(Value::Quantity { value: "0.0000001".into(), unit: "F".into() })
        );
        set_value(&mut session, "220n");
        if mode == 2 {
            apply(&mut session, crate::sheets::CREATE_SHEET, json!({"name":"Keep"}));
        }
        shown(session.project().unwrap(), "220 nF", false);
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        let symbol = session.project().unwrap().symbols()[0].id();
        let sheet = session.project().unwrap().sheet_target().id();
        let (command, params) = match mode {
            0 => (crate::DELETE_SYMBOL, json!({"symbol":symbol})),
            1 => (crate::DELETE_ITEMS, json!({"items":{"symbols":[symbol],"wires":[],"net_labels":[]}})),
            _ => (crate::sheets::DELETE_SHEET, json!({"sheet":sheet})),
        };
        apply(&mut session, command, params);
        assert!(session.project().unwrap().components().next().is_none());
        let revision = session.revision();
        let snapshot = session.snapshot().unwrap();
        shown(&snapshot, "220 nF", true);
        let inventory = crate::library::sources::inventory::Inventory::from_project(&snapshot).unwrap();
        assert!(inventory.entries().any(|entry| entry.item.key.kind == sources::Kind::Footprint && entry.used_by.contains("Accepted board history")));
        let saved = encode_documents(snapshot.clone()).unwrap();
        let packed = encode_packed(snapshot.clone()).unwrap();
        let unpacked = decode_packed(&packed).unwrap().project;
        assert_eq!(encode_packed(unpacked.clone()).unwrap(), packed);
        for reopened in [decode_documents(&saved).unwrap(), unpacked] {
            shown(&reopened, "220 nF", true);
            assert_eq!(encode_documents(reopened).unwrap(), saved);
        }
        session.inspect();
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
        session.apply("edit.undo", &Json::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
        session.apply("edit.redo", &Json::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
        accept(&mut session);
        assert_eq!(session.project().unwrap().boards().next().unwrap().footprints().count(), 0);
        session.apply("edit.undo", &Json::Null).unwrap();
        shown(session.project().unwrap(), "220 nF", true);
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    }
}

/// Fixed primary parameters are captured too; typed-but-unset and plain text stay distinct from zero.
#[test]
fn retained_fixed_unset_and_untyped_values_preserve_their_meaning() {
    let mut fixed = fixture(true);
    let params = placement(&fixed, false);
    apply(&mut fixed, PLACE_PART, params);
    let id = fixed.project().unwrap().components().next().unwrap().id();
    apply(&mut fixed, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":id,"x_nm":0,"y_nm":0}));
    apply(&mut fixed, INITIALIZE_BASELINE, json!({}));
    assert!(fixed.project().unwrap().board.baseline.as_ref().unwrap().components.values().next().unwrap().primary_quantity.is_some());
    let symbol = fixed.project().unwrap().symbols()[0].id();
    apply(&mut fixed, crate::DELETE_SYMBOL, json!({"symbol":symbol}));
    shown(fixed.project().unwrap(), "100 mΩ", true);

    let mut unset = typed_part(Quantity::Resistance, None);
    apply(&mut unset, INITIALIZE_BASELINE, json!({}));
    let baseline = unset.project().unwrap().board.baseline.as_ref().unwrap();
    assert_eq!(baseline.format, "pcbcraft-board-baseline/1");
    assert!(baseline.components.values().next().unwrap().primary_quantity.is_none());

    let mut plain = Session::default();
    plain.apply("project.new", &json!({"create_board":true})).unwrap();
    apply(&mut plain, crate::PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    let id = plain.project().unwrap().components().next().unwrap().id();
    let package = plain.project().unwrap().footprint_catalog().into_iter().find(|package| package.id == "resistor-0603").unwrap();
    apply(
        &mut plain,
        crate::ASSIGN_FOOTPRINT,
        json!({"component":id,"library":package.library,
        "definition":package.id,"definition_revision":package.revision,"mapping":{"1":"1","2":"2"}}),
    );
    apply(&mut plain, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":id,"x_nm":0,"y_nm":0}));
    let symbol = plain.project().unwrap().symbols()[0].id();
    apply(&mut plain, crate::SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":"R1","value":"0.5 V"}));
    apply(&mut plain, INITIALIZE_BASELINE, json!({}));
    apply(&mut plain, crate::DELETE_SYMBOL, json!({"symbol":symbol}));
    shown(plain.project().unwrap(), "0.5 V", true);
    assert!(!String::from_utf8(plain.project().unwrap().board.baseline.as_ref().unwrap().encode().unwrap()).unwrap().contains("primary_quantity"));
}

/// Old typed baselines keep their exact encoding through inspection and no-op Apply.
/// A real accepted change adopts format 3; already-deleted old parts retain text.
#[test]
fn legacy_baselines_adopt_quantities_only_on_real_acceptance() {
    for version in ["pcbcraft-board-baseline/1", "pcbcraft-board-baseline/2"] {
        let mut session = typed_part(Quantity::Capacitance, Some("100n"));
        apply(&mut session, INITIALIZE_BASELINE, json!({}));
        let mut historical = session.project().unwrap().clone();
        let baseline = Arc::make_mut(Arc::make_mut(&mut historical.board).baseline.as_mut().unwrap());
        baseline.format = version.into();
        baseline.components.values_mut().for_each(|part| part.primary_quantity = None);
        let saved = encode_documents(Arc::new(historical)).unwrap();
        let packed = encode_packed(decode_documents(&saved).unwrap()).unwrap();
        let unpacked = decode_packed(&packed).unwrap().project;
        assert_eq!(encode_packed(unpacked.clone()).unwrap(), packed);
        for reopened in [decode_documents(&saved).unwrap(), unpacked] {
            assert_eq!(encode_documents(reopened).unwrap(), saved);
        }
        session.open_snapshot(decode_documents(&saved).unwrap());
        let before = session.snapshot().unwrap();
        let revision = session.revision();
        accept(&mut session);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
        let symbol = session.project().unwrap().symbols()[0].id();
        apply(&mut session, crate::DELETE_SYMBOL, json!({"symbol":symbol}));
        shown(session.project().unwrap(), "0.0000001 F", true);
        let deleted = encode_documents(session.snapshot().unwrap()).unwrap();
        shown(&decode_documents(&deleted).unwrap(), "0.0000001 F", true);
        assert_eq!(encode_documents(decode_documents(&deleted).unwrap()).unwrap(), deleted);
        session.apply("edit.undo", &Json::Null).unwrap();
        let params = placement(&session, false);
        apply(&mut session, PLACE_PART, params);
        accept(&mut session);
        let baseline = session.project().unwrap().board.baseline.as_ref().unwrap();
        assert_eq!(baseline.format, "pcbcraft-board-baseline/3");
        assert!(baseline.components.values().next().unwrap().primary_quantity.is_some());
        apply(&mut session, crate::DELETE_SYMBOL, json!({"symbol":symbol}));
        shown(session.project().unwrap(), "100 nF", true);
    }
}

/// Validate canonical quantities without live categories, rejecting incompatible vocabulary,
/// arbitrary typed values, mismatched text, explicit nulls and malformed units/decimals.
#[test]
fn retained_primary_quantity_codec_rejects_invalid_or_legacy_metadata() {
    let mut session = typed_part(Quantity::Capacitance, Some("100n"));
    apply(&mut session, INITIALIZE_BASELINE, json!({}));
    let baseline = session.project().unwrap().board.baseline.as_ref().unwrap();
    let encoded = baseline.encode().unwrap();
    assert_eq!(Baseline::decode(&encoded).unwrap(), **baseline);
    let original: Json = serde_json::from_slice(&encoded).unwrap();
    let id = original["components"].as_object().unwrap().keys().next().unwrap().clone();
    for bad in [
        Json::Null,
        json!({"type":"text","value":"0.0000001 F"}),
        json!({"type":"quantity","value":"1e-7","unit":"F"}),
        json!({"type":"quantity","value":"0.0000001","unit":"nF"}),
        json!({"type":"quantity","value":"0.0000002","unit":"F"}),
        json!({"type":"quantity","value":"0.0000001","unit":"F","extra":true}),
    ] {
        let mut invalid = original.clone();
        invalid["components"][&id]["primary_quantity"] = bad;
        assert!(Baseline::decode(&serde_json::to_vec(&invalid).unwrap()).is_err(), "{invalid}");
    }
    for version in ["pcbcraft-board-baseline/1", "pcbcraft-board-baseline/2", "pcbcraft-board-baseline/4"] {
        let mut invalid = original.clone();
        invalid["format"] = json!(version);
        assert!(Baseline::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}
