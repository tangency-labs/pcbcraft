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
    assert!(sources.as_array().unwrap().iter().any(|s| s["library"] == "basic-components" && s["name"] == "Basic components"));
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
    let mut ordinary: Value = serde_json::from_str(include_str!("../../../../libraries/basic-components/symbols/resistor.json")).unwrap();
    ordinary["name"] = json!("GND");
    assert!(parse_symbol(&ordinary.to_string()).unwrap().power_terminal().is_none());
}

/// Bundled definitions participate in identical-copy collapse and immutable-revision quarantine.
/// Omitted-library compatibility cannot bypass conflicts, and bundled registrations resist removal.
#[test]
fn bundled_selections_obey_normal_conflicts_and_content_authority() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let (_, mut docs) = bundled_documents().find(|(id, _)| *id == "basic-components").unwrap();
    s.execute(sources::LOAD, &json!({"source":"another-copy","documents":docs})).unwrap();
    assert_eq!(s.libraries().search("resistor", Some(sources::Kind::Symbol)).into_iter().filter(|(_, item)| item.key.id == "resistor").count(), 1);
    let before = s.snapshot().unwrap();
    let mut symbol: Value = serde_json::from_str(&docs["symbols/resistor.json"]).unwrap();
    symbol["default_value"] = json!("Conflicting");
    docs.insert("symbols/resistor.json".into(), symbol.to_string());
    s.execute(sources::LOAD, &json!({"source":"another-copy","documents":docs})).unwrap();
    assert!(s.libraries().latest_symbol("basic-components", "resistor").is_none());
    assert!(s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.execute(sources::REMOVE, &json!({"source":"another-copy"})).unwrap();
    assert!(s.libraries().latest_symbol("basic-components", "resistor").is_some());
    assert!(s.execute(sources::REMOVE, &json!({"source":"bundled-basic-components"})).unwrap_err().to_string().contains("read-only"));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
}

/// A digest opts into exact selection even when the caller relies on the default library ID.
/// Changed content from a replaceable external registration is refused without changing history.
#[test]
fn digest_without_library_cannot_bypass_pinned_content_authority() {
    let mut s = Session { libraries: Arc::new(sources::Catalog::default()), ..Session::default() };
    s.apply("project.new", &json!({})).unwrap();
    let (_, mut docs) = bundled_documents().find(|(id, _)| *id == "basic-components").unwrap();
    s.execute(sources::LOAD, &json!({"source":"external-starter","documents":docs})).unwrap();
    place(&mut s, "basic-components", "resistor", 0, 0, "1k");
    let before = s.snapshot().unwrap();
    let state = s.inspect();
    let mut symbol: Value = serde_json::from_str(&docs["symbols/resistor.json"]).unwrap();
    symbol["default_value"] = json!("Changed source");
    docs.insert("symbols/resistor.json".into(), symbol.to_string());
    s.execute(sources::REMOVE, &json!({"source":"external-starter"})).unwrap();
    s.execute(sources::LOAD, &json!({"source":"replacement","documents":docs})).unwrap();
    let d = s.libraries().latest_symbol("basic-components", "resistor").unwrap();
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
    let d = s.libraries().latest_symbol("basic-components", "resistor").unwrap();
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
    place(&mut s, "basic-components", "resistor", 10 * GRID_NM, 0, "1k");
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
    assert_eq!(entry["format"], "pcbcraft-project/52");
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

/// Connector pins face left and number contacts from top to bottom without changing their
/// attachment positions. The two-row header preserves its alternating numbering.
#[test]
fn revised_connectors_keep_their_pinned_connection_points() {
    let library = basic_components();
    for (id, count) in [("connector", 2), ("header-3", 3), ("header-4", 4), ("header-6", 6), ("header-8", 8)] {
        let definition = library.symbol(id).unwrap();
        assert_eq!(definition.pins().len(), count);
        for (index, pin) in definition.pins().iter().enumerate() {
            assert_eq!(pin.number, (index + 1).to_string());
            assert_eq!(
                pin.offset,
                Point { x_nm: -3 * GRID_NM, y_nm: (2 * index as i64 - count as i64 + 1) * GRID_NM * if count == 6 { 2 } else { 1 } }
            );
            let appearance = pin.appearance.as_ref().unwrap();
            assert_eq!(appearance.orientation, crate::schematic::PinOrientation::Right);
            assert!(appearance.show_number);
        }
    }
    let dual = library.symbol("header-2x3").unwrap();
    for row in 0..3 {
        assert_eq!(dual.pins()[2 * row].offset, Point { x_nm: -4 * GRID_NM, y_nm: (4 * row as i64 - 4) * GRID_NM });
        assert_eq!(dual.pins()[2 * row + 1].offset, Point { x_nm: 4 * GRID_NM, y_nm: (4 * row as i64 - 4) * GRID_NM });
    }
}

/// Old and new exact revisions coexist, wire by stable terminal identity and survive native
/// offline reopen without substituting current geometry, power defaults or authored values.
#[test]
fn changed_bundled_revisions_reopen_offline_without_upgrading_instances() {
    for (library, id, revision) in [
        ("basic-components", "inductor", 1),
        ("basic-components", "nmos", 1),
        ("basic-components", "pmos", 1),
        ("power", "3v3", 1),
        ("power", "5v", 1),
        ("power", "12v", 1),
        ("power", "vdd", 1),
        ("power", "vss", 1),
        ("power", "gnda", 1),
    ] {
        let mut session = Session::default();
        session.apply("project.new", &json!({})).unwrap();
        let old = session.libraries().symbol(library, id, revision).unwrap();
        let latest = session.libraries().latest_symbol(library, id).unwrap();
        assert!(latest.revision() > revision);
        assert_ne!(old.default_unit().graphics(), latest.default_unit().graphics());
        for (index, definition) in [old, latest].into_iter().enumerate() {
            session.apply(PLACE_SYMBOL, &json!({"library":library,"definition":id,"definition_revision":definition.revision(),"definition_digest":sources::symbol_digest(&definition).unwrap(),"x_nm":index as i64*20*GRID_NM,"y_nm":0,"expected_revision":session.revision()})).unwrap();
        }
        let before = session.snapshot().unwrap();
        let a = &before.symbols()[0];
        let b = &before.symbols()[1];
        session
            .apply(
                crate::CONNECT_PINS,
                &json!({"from":{"symbol":a.id(),"terminal":"1"},"to":{"symbol":b.id(),"terminal":"1"},"expected_revision":session.revision()}),
            )
            .unwrap();
        let before = session.snapshot().unwrap();
        let documents = encode_documents(before.clone()).unwrap();
        let mut offline = Session { libraries: Arc::new(sources::Catalog::default()), ..Session::default() };
        offline.open_snapshot(decode_documents(&documents).unwrap());
        let after = offline.snapshot().unwrap();
        for (a, b) in before.symbols().iter().zip(after.symbols()) {
            assert_eq!(a.definition, b.definition);
            assert_eq!(a.pins(), b.pins());
            assert_eq!(a.field_layout, b.field_layout);
        }
        assert_eq!(encode_documents(after).unwrap(), documents);
    }
}

/// New ground drawings keep deliberately distinct global names; voltage and complementary
/// supply redraws preserve their electrical names and attach at the same origin as revision 1.
#[test]
fn expanded_power_shapes_preserve_explicit_net_semantics() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    for (index, id) in ["gnd", "gnda", "earth", "chassis", "3v3", "5v", "12v", "vcc", "vdd", "vss"].into_iter().enumerate() {
        let definition = session.libraries().latest_symbol("power", id).unwrap();
        assert_eq!(definition.pins()[0].offset, Point { x_nm: 0, y_nm: 0 });
        place(&mut session, "power", id, (index % 5) as i64 * 20 * GRID_NM, (index / 5) as i64 * 20 * GRID_NM, definition.default_value());
    }
    let project = session.snapshot().unwrap();
    let graph = project.electrical_connectivity();
    let mut nets = std::collections::BTreeSet::new();
    for (symbol, name) in project.symbols().iter().zip(["GND", "GNDA", "PE", "CHASSIS", "+3V3", "+5V", "+12V", "VCC", "VDD", "VSS"]) {
        let net = graph.net_index_for(symbol.terminals()[0]).unwrap();
        assert_eq!(graph.net_name(net), name);
        assert!(nets.insert(net));
    }
}

/// Connector bodies contain only their outline; numbered integrated leads stop at its edge. Coil
/// turns remain editable semicircular arcs; MOSFET attachment leads follow their direct axes.
#[test]
fn corrected_native_glyphs_keep_clear_numbering_coils_and_straight_leads() {
    use crate::library_graphics::Geometry;
    use crate::schematic::PinOrientation;
    for id in ["connector", "header-3", "header-4", "header-6", "header-8", "header-2x3"] {
        let symbol = basic_components().symbol(id).unwrap();
        assert_eq!(symbol.default_unit().drawing.len(), 1, "{id} contains extra body strokes");
        let Shape::Rect(min, max) = symbol.default_unit().drawing[0] else { panic!("{id} body is not a rectangle") };
        for pin in symbol.pins() {
            let appearance = pin.appearance.as_ref().unwrap();
            let body_x = match appearance.orientation {
                PinOrientation::Right => pin.offset.x_nm + appearance.length_nm,
                PinOrientation::Left => pin.offset.x_nm - appearance.length_nm,
                _ => panic!("{id} lead is not horizontal"),
            };
            assert!(body_x == min.x_nm || body_x == max.x_nm, "{id} lead must stop at body edge");
            assert!(pin.offset.y_nm > min.y_nm && pin.offset.y_nm < max.y_nm);
        }
    }
    let coil = basic_components().symbol("inductor").unwrap();
    let turns: Vec<_> = coil
        .default_unit()
        .drawing
        .iter()
        .filter_map(|s| match s {
            Shape::Graphic(g) => Some(g),
            _ => None,
        })
        .collect();
    assert_eq!(turns.len(), 4);
    assert!(turns.iter().all(|g| matches!(g.geometry, Geometry::Arc { sweep_degrees: 180, .. })));
    for pair in turns.windows(2) {
        assert_eq!(pair[0].paths()[0].last(), pair[1].paths()[0].first());
    }
    for id in ["nmos", "pmos"] {
        let symbol = basic_components().symbol(id).unwrap();
        let gate = &symbol.pins()[0];
        let source = &symbol.pins()[1];
        let drain = &symbol.pins()[2];
        assert_eq!((gate.offset.y_nm, source.offset.x_nm, drain.offset.x_nm), (0, 0, 0));
        for (pin, direction) in [(gate, PinOrientation::Right), (source, PinOrientation::Up), (drain, PinOrientation::Down)] {
            assert_eq!(pin.appearance.as_ref().unwrap().orientation, direction);
        }
    }
}
