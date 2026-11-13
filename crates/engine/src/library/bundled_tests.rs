//! Original command-built circuits exercise delivery-independent native symbol behavior.
use super::*;
use crate::{
    MOVE_SYMBOL, PLACE_SYMBOL, SET_SYMBOL_FIELDS, Session,
    persistence::{decode_documents, encode_documents},
};
use serde_json::json;

/// Place the exact source selection, retaining the same content authority used by the GUI.
fn place(s: &mut Session, library: &str, id: &str, x: i64, y: i64, value: &str) {
    let d = s.libraries().latest_symbol(library, id).unwrap();
    s.apply(PLACE_SYMBOL,&json!({"library":library,"definition":id,"definition_revision":d.revision(),"definition_digest":sources::symbol_digest(&d).unwrap(),"value":value,"x_nm":x,"y_nm":y,"expected_revision":s.revision()})).unwrap();
}

/// Every bundled folder passes the public folder/bundle loader, including attribution/digests.
#[test]
fn embedded_and_external_documents_are_the_same_native_items() {
    let s = Session::default();
    for (id, docs) in bundled_documents() {
        let folder = sources::NativeSource::load(&docs).unwrap();
        assert!(folder.problems().is_empty(), "{id}: {:?}", folder.problems());
        let bundle = json!({"format":"pcbcraft-library-bundle/1","documents":docs}).to_string();
        let uploaded = sources::NativeSource::from_bundle(bundle.as_bytes()).unwrap();
        assert_eq!(folder.items().len(), uploaded.items().len());
        for (a, b) in folder.items().iter().zip(uploaded.items()) {
            assert_eq!(a.inspect(), b.inspect());
            if let Some(symbol) = a.symbol() {
                let embedded = s.libraries().symbol(&symbol.library, &symbol.id, symbol.revision()).unwrap();
                assert_eq!(symbol, embedded);
                assert_eq!(a.inspect()["digest"], sources::symbol_digest(&embedded).unwrap());
            }
        }
    }
    let sources = s.libraries().inspect();
    assert!(sources.as_array().unwrap().iter().any(|s| s["library"] == "starter" && s["name"] == "Default"));
}

/// Arbitrary library and item identities carry power semantics through normal component fields,
/// common pose edits, history, offline pinning, renaming and independent sheet scope.
#[test]
fn custom_native_roles_use_normal_instances_and_global_values() {
    let (_, mut docs) = bundled_documents().find(|(id, _)| *id == "power").unwrap();
    let mut manifest: Value = serde_json::from_str(&docs["library.json"]).unwrap();
    manifest["id"] = json!("rails");
    manifest["name"] = json!("Custom rails");
    docs.insert("library.json".into(), manifest.to_string());
    let mut symbol: Value = serde_json::from_str(&docs["symbols/gnd.json"]).unwrap();
    symbol["id"] = json!("unrelated-token");
    docs.retain(|path, _| path == "library.json");
    docs.insert("symbols/drawing.json".into(), symbol.to_string());
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.execute(sources::LOAD, &json!({"source":"custom","documents":docs})).unwrap();
    place(&mut s, "rails", "unrelated-token", 0, 0, "QUIET_RETURN");
    place(&mut s, "power", "gnd", 20 * GRID_NM, 0, "QUIET_RETURN");
    let p = s.snapshot().unwrap();
    assert!(p.net_labels().is_empty());
    assert_eq!(p.components().count(), 2);
    let a = p.symbols()[0].clone();
    let b = &p.symbols()[1];
    let graph = p.electrical_connectivity();
    assert_eq!(graph.net_index_for(a.terminals()[0]), graph.net_index_for(b.terminals()[0]));
    assert_eq!(graph.net_name(graph.net_index_for(a.terminals()[0]).unwrap()), "QUIET_RETURN");
    let c = p.component(&a).unwrap();
    assert!(c.metadata.attributes.exclude_from_board);
    assert!(!a.field_layout["reference"].visible);
    let mut layout = a.field_layout.clone();
    layout.get_mut("value").unwrap().offset = Some(Point { x_nm: 5 * GRID_NM, y_nm: 3 * GRID_NM });
    s.apply(
        SET_SYMBOL_FIELDS,
        &json!({"symbol":a.id(),"reference":c.reference(),"value":"ANOTHER_RETURN","field_layout":layout,"expected_revision":s.revision()}),
    )
    .unwrap();
    let edited = s.snapshot().unwrap();
    assert_ne!(edited.electrical_connectivity().net_index_for(a.terminals()[0]), edited.electrical_connectivity().net_index_for(b.terminals()[0]));
    s.apply(MOVE_SYMBOL, &json!({"symbol":a.id(),"x_nm":2*GRID_NM,"y_nm":3*GRID_NM,"rotation":90,"expected_revision":s.revision()})).unwrap();
    let bytes = encode_documents(s.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&bytes).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), bytes);
    assert_eq!(reopened.symbols()[0].definition.power_terminal(), Some("1"));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), encode_documents(edited).unwrap());
    s.execute(sources::REMOVE, &json!({"source":"custom"})).unwrap();
    assert_eq!(reopened.component(&reopened.symbols()[0]).unwrap().value(), "ANOTHER_RETURN");
}

/// Wire-first contacts survive the ordinary connected move and rotation; M leaves the wire
/// behind. Preview and command agree in both cleanup modes, including a cached named graph.
#[test]
fn native_power_contacts_follow_the_shared_move_command() {
    for auto_heal in [false, true] {
        for disconnect in [false, true] {
            let mut s = Session::default();
            s.apply("project.new", &json!({})).unwrap();
            s.apply(
                crate::DRAW_WIRE,
                &json!({"from":{"x_nm":-10*GRID_NM,"y_nm":0},"to":{"x_nm":0,"y_nm":0},"bends":[],"expected_revision":s.revision()}),
            )
            .unwrap();
            place(&mut s, "power", "gnd", 0, 0, "GND");
            let p = s.snapshot().unwrap();
            let sym = &p.symbols()[0];
            let end = sym.terminals()[0];
            let _ = p.electrical_connectivity();
            let to = Point { x_nm: 4 * GRID_NM, y_nm: 4 * GRID_NM };
            let preview = p.transformed_symbol(&sym.id(), to, Some(1), None, disconnect, s.revision(), auto_heal).unwrap();
            s.apply(MOVE_SYMBOL,&json!({"symbol":sym.id(),"x_nm":to.x_nm,"y_nm":to.y_nm,"rotation":90,"disconnect":disconnect,"auto_heal":auto_heal,"expected_revision":s.revision()})).unwrap();
            let after = s.snapshot().unwrap();
            assert_eq!(encode_documents(after.clone()).unwrap(), encode_documents(Arc::new(preview)).unwrap());
            let path = after.wire_path(&after.wires()[0]);
            assert_eq!(*path.last().unwrap(), if disconnect { Point { x_nm: 0, y_nm: 0 } } else { to });
            assert_eq!(after.electrical_connectivity().net_name(after.electrical_connectivity().net_index_for(end).unwrap()), "GND");
            s.apply("edit.undo", &Value::Null).unwrap();
            crate::assert_same_content(&p, &s.snapshot().unwrap());
        }
    }
}

/// Definitions never infer electrical behavior from names, and invalid roles/defaults are isolated.
#[test]
fn roles_and_defaults_are_versioned_validated_and_preserved_by_authoring() {
    let (_, docs) = bundled_documents().find(|(id, _)| *id == "power").unwrap();
    let mut v: Value = serde_json::from_str(&docs["symbols/gnd.json"]).unwrap();
    let d = Session::default().libraries().latest_symbol("power", "gnd").unwrap();
    let item = crate::authoring::Item::from_symbol(&d).unwrap();
    assert_eq!(item.symbol().unwrap().behavior, d.behavior);
    assert_eq!(item.symbol().unwrap().defaults, d.defaults);
    let exported = parse_symbol(&item.export_native().unwrap().to_string()).unwrap();
    assert_eq!(exported.behavior, d.behavior);
    assert_eq!(exported.defaults, d.defaults);
    assert_eq!(crate::authoring::Item::decode(&serde_json::to_vec(&item).unwrap()).unwrap(), item);
    v["behavior"]["terminal"] = json!("missing");
    assert!(parse_symbol(&v.to_string()).unwrap_err().contains("existing terminal"));
    v["behavior"]["terminal"] = json!("1");
    v["format"] = json!(SYMBOL_FORMAT);
    assert!(parse_symbol(&v.to_string()).is_err());
    v["format"] = json!(SYMBOL_FORMAT_V2);
    v["defaults"]["field_layout"]["value"]["height_nm"] = json!(0);
    assert!(parse_symbol(&v.to_string()).is_err());
    let mut ordinary: Value = serde_json::from_str(include_str!("../../../../libraries/starter/symbols/resistor.json")).unwrap();
    ordinary["name"] = json!("GND");
    assert!(parse_symbol(&ordinary.to_string()).unwrap().power_terminal().is_none());
}

/// Bundled definitions participate in identical-copy collapse and immutable-revision quarantine.
/// Omitted-library compatibility cannot bypass a conflicting or removed catalog definition.
#[test]
fn bundled_selections_obey_normal_conflicts_and_content_authority() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let (_, mut docs) = bundled_documents().find(|(id, _)| *id == "starter").unwrap();
    s.execute(sources::LOAD, &json!({"source":"another-copy","documents":docs})).unwrap();
    assert_eq!(s.libraries().search("resistor", Some(sources::Kind::Symbol)).len(), 1);
    let before = s.snapshot().unwrap();
    let mut symbol: Value = serde_json::from_str(&docs["symbols/resistor.json"]).unwrap();
    symbol["default_value"] = json!("Conflicting");
    docs.insert("symbols/resistor.json".into(), symbol.to_string());
    s.execute(sources::LOAD, &json!({"source":"another-copy","documents":docs})).unwrap();
    assert!(s.libraries().latest_symbol("starter", "resistor").is_none());
    assert!(s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.execute(sources::REMOVE, &json!({"source":"another-copy"})).unwrap();
    assert!(s.libraries().latest_symbol("starter", "resistor").is_some());
    s.execute(sources::REMOVE, &json!({"source":"bundled-starter"})).unwrap();
    assert!(s.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// A digest opts into exact selection even when the caller relies on the default library ID.
/// Changed source content at the pinned revision is refused without changing history.
#[test]
fn digest_without_library_cannot_bypass_pinned_content_authority() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    place(&mut s, "starter", "resistor", 0, 0, "1k");
    let before = s.snapshot().unwrap();
    let state = s.inspect();
    let (_, mut docs) = bundled_documents().find(|(id, _)| *id == "starter").unwrap();
    let mut symbol: Value = serde_json::from_str(&docs["symbols/resistor.json"]).unwrap();
    symbol["default_value"] = json!("Changed source");
    docs.insert("symbols/resistor.json".into(), symbol.to_string());
    s.execute(sources::REMOVE, &json!({"source":"bundled-starter"})).unwrap();
    s.execute(sources::LOAD, &json!({"source":"replacement","documents":docs})).unwrap();
    let d = s.libraries().latest_symbol("starter", "resistor").unwrap();
    let params = json!({"definition":"resistor","definition_revision":d.revision(),"definition_digest":sources::symbol_digest(&d).unwrap(),"x_nm":10*GRID_NM,"y_nm":0,"expected_revision":s.revision()});
    assert!(s.apply(PLACE_SYMBOL, &params).unwrap_err().to_string().contains("pinned project copy"));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(state, s.inspect());
}

/// A revision alone is an exact selection and requires its content digest, while unqualified
/// calls keep the default-library compatibility path. Refusal does not author a partial edit.
#[test]
fn explicit_revision_requires_digest_even_without_library() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let before = s.snapshot().unwrap();
    let state = s.inspect();
    let d = s.libraries().latest_symbol("starter", "resistor").unwrap();
    let mut params = json!({"definition":"resistor","definition_revision":d.revision(),"x_nm":0,"y_nm":0,"expected_revision":s.revision()});
    let error = s.apply(PLACE_SYMBOL, &params).unwrap_err();
    assert!(error.to_string().contains("`definition_digest` is required"));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(s.inspect(), state);
    params["definition_digest"] = json!(sources::symbol_digest(&d).unwrap());
    s.apply(PLACE_SYMBOL, &params).unwrap();
    params.as_object_mut().unwrap().remove("definition_revision");
    params.as_object_mut().unwrap().remove("definition_digest");
    params["expected_revision"] = json!(s.revision());
    params["x_nm"] = json!(10 * GRID_NM);
    s.apply(PLACE_SYMBOL, &params).unwrap();
    assert_eq!(s.project().unwrap().symbols().len(), 2);
}

/// Native power defaults and pinned definitions coexist with schematic-only package intent.
/// Offline round trips preserve both, and first-board creation adopts only the chosen package.
#[test]
fn native_power_roundtrips_with_schematic_only_package_choices() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    place(&mut s, "power", "gnd", 0, 0, "RETURN");
    place(&mut s, "starter", "resistor", 10 * GRID_NM, 0, "1k");
    let p = s.project().unwrap();
    let component = p.component(&p.symbols()[1]).unwrap().id();
    let footprint = p.footprint_catalog().into_iter().find(|d| d.id == "resistor-0402").unwrap();
    s.apply(
        crate::ASSIGN_FOOTPRINT,
        &json!({"component":component,"library":footprint.library,"definition":footprint.id,"definition_revision":footprint.revision,
            "mapping":{"1":"2","2":"1"},"expected_revision":s.revision()}),
    )
    .unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&saved["project.json"]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/17");
    assert!(!saved.contains_key("board.json"));
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
    assert_eq!(reopened.power_terminals().map(|(_, name, _)| name).collect::<Vec<_>>(), ["RETURN"]);
    assert!(reopened.component(&reopened.symbols()[0]).unwrap().metadata.attributes.exclude_from_board);
    s.open_snapshot(reopened);
    s.apply(crate::board::identity::CREATE_BOARD, &json!({"expected_revision":s.revision()})).unwrap();
    assert_eq!(s.project().unwrap().footprint_assignments().len(), 1);
    assert!(s.project().unwrap().board.footprints().is_empty());
    let with_board = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&with_board).unwrap()).unwrap(), with_board);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}
