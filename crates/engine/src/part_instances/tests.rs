//! Original native fixtures exercise the full selection/placement boundary, independent of vendors.
use super::*;
use crate::{
    Session,
    library::sources,
    part_instances::{PLACE_PART, SET_PART},
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::json;

/// Transport the same original native source data that folder and browser bundle loading use.
fn documents() -> sources::Documents {
    [
        ("library.json", include_str!("../../../../examples/native-parts/library.json")),
        ("symbols/resistor.json", include_str!("../../../../examples/native-parts/symbols/resistor.json")),
        ("footprints/resistor-0603.json", include_str!("../../../../examples/native-parts/footprints/resistor-0603.json")),
        ("categories/resistor.json", include_str!("../../../../examples/native-parts/categories/resistor.json")),
        ("parts/resistor.json", include_str!("../../../../examples/native-parts/parts/resistor.json")),
        ("manufacturer-parts/test-resistor.json", include_str!("../../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json")),
    ]
    .into_iter()
    .map(|(path, text)| (path.into(), text.into()))
    .collect()
}
/// All project edits use the ordinary revision-checked shared commands.
fn apply(session: &mut Session, command: &str, mut params: Json) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}
/// A complete native part may be placed without a physical board or with one already registered.
fn fixture(board: bool) -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":board})).unwrap();
    let mut documents = documents();
    let mut part: Json = serde_json::from_str(&documents["parts/resistor.json"]).unwrap();
    part["fixed_parameters"] = json!({"pcbcraft:resistance":{"type":"quantity","value":"0.1","unit":"Ohm"}});
    documents.insert("parts/resistor.json".into(), part.to_string());
    let source = session.execute(sources::LOAD, &json!({"source":"native-parts","documents":documents})).unwrap();
    let loaded = source.as_array().unwrap().iter().find(|source| source["source"] == "native-parts").unwrap();
    assert_eq!(loaded["problems"], json!([]));
    session
}
/// Freeze a source-backed placement selection using the same digest exposed by library inspection.
fn placement(session: &Session, manufacturer: bool) -> Json {
    let reference = Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
    let resolved = session.libraries.resolve_part(&reference).unwrap();
    let mut params = json!({"library":"parts-demo","definition":"resistor","definition_revision":1,"definition_digest":sources::part_digest(resolved),"x_nm":0,"y_nm":0});
    if manufacturer {
        let record = session.libraries.manufacturers_for(&reference).pop().unwrap();
        params["manufacturer"] = json!(record.reference());
        params["manufacturer_digest"] = json!(sources::manufacturer_digest(&record));
    }
    params
}

/// Central part, category, package and MPN records pin together and reopen with no sources loaded.
#[test]
fn native_part_selection_is_atomic_offline_and_shared_by_its_package() {
    for board in [false, true] {
        let mut session = fixture(board);
        let before = session.snapshot().unwrap();
        let params = placement(&session, true);
        apply(&mut session, PLACE_PART, params);
        let project = session.snapshot().unwrap();
        let instance = project.components().next().unwrap();
        assert_eq!(instance.value(), "0.1 Ohm");
        let selected = instance.part.as_ref().unwrap();
        assert_eq!(selected.package.as_deref(), Some("0603"));
        assert_eq!(selected.manufacturer.as_ref().unwrap().mpn, "TEST-R100M-0603");
        assert_eq!(project.footprint_assignments().len(), 1);
        let docs = encode_documents(project.clone()).unwrap();
        let header: Json = serde_json::from_slice(&docs[crate::persistence::ENTRY_POINT]).unwrap();
        assert_eq!(header["format"], "pcbcraft-project/16");
        // Isolate the older vocabulary: remove occurrence records through the historical codec
        // before forging the format header, so only native part selections cause this rejection.
        let mut reserved_format = encode_documents(crate::persistence::diagnostic_snapshot(project.clone())).unwrap();
        let mut reserved: Json = serde_json::from_slice(&reserved_format[crate::persistence::ENTRY_POINT]).unwrap();
        reserved["format"] = json!("pcbcraft-project/14");
        reserved_format.insert(crate::persistence::ENTRY_POINT.into(), serde_json::to_vec(&reserved).unwrap());
        let error = decode_documents(&reserved_format).unwrap_err();
        assert!(error.contains("Pinned part selections require project format 15, 16 or 17"), "unexpected rejection: {error}");
        assert!(docs.keys().any(|path| path.starts_with("library/parts/")));
        assert!(docs.keys().any(|path| path.starts_with("library/categories/")));
        assert!(docs.keys().any(|path| path.starts_with("library/manufacturer-parts/")));
        let reopened = decode_documents(&docs).unwrap();
        assert_eq!(encode_documents(reopened).unwrap(), docs);
        assert_eq!(encode_documents(decode_packed(&encode_packed(project.clone()).unwrap()).unwrap().project).unwrap(), docs);
        session.apply("edit.undo", &serde_json::Value::Null).unwrap();
        assert!(session.project().unwrap().components().next().is_none());
        assert!(before.components().next().is_none());
        session.apply("edit.redo", &serde_json::Value::Null).unwrap();
        session.execute(sources::REMOVE, &json!({"source":"native-parts"})).unwrap();
        if board {
            let component = session.project().unwrap().components().next().unwrap().id();
            apply(&mut session, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":0,"y_nm":0}));
            assert_eq!(session.project().unwrap().board.footprints().len(), 1);
            let docs = encode_documents(session.snapshot().unwrap()).unwrap();
            assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
        }
    }
}

/// Requirement edits retain the chosen MPN and expose incompatibility; choosing an unsuitable MPN is refused.
#[test]
fn manufacturer_specifications_are_distinct_from_instance_requirements() {
    let mut session = fixture(true);
    let params = placement(&session, true);
    apply(&mut session, PLACE_PART, params);
    let part = session.project().unwrap().components().next().unwrap().id();
    apply(
        &mut session,
        SET_PART,
        json!({"component":part,"requirements":{"pcbcraft:tolerance":{"operator":"at_most","value":{"type":"quantity","value":"0.005","unit":"1"}}}}),
    );
    let selected = session.project().unwrap().components().next().unwrap().part.as_ref().unwrap();
    assert!(selected.manufacturer.is_some());
    assert_eq!(selected.compatibility()["status"], "does_not_meet_requirements");
    let before = session.snapshot().unwrap();
    let mut params = placement(&session, true);
    params["requirements"] = json!({"pcbcraft:tolerance":{"operator":"at_most","value":{"type":"quantity","value":"0.005","unit":"1"}}});
    params["expected_revision"] = json!(session.revision());
    assert!(session.apply(PLACE_PART, &params).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// Canonical equivalent unit edits preserve part identity and fixed requirements; conflicting edits are atomic.
#[test]
fn primary_value_uses_canonical_units_and_fixed_part_values() {
    let mut session = fixture(true);
    let params = placement(&session, false);
    apply(&mut session, PLACE_PART, params);
    let symbol = session.project().unwrap().symbols()[0].id();
    let revision = session.revision();
    let shown = {
        let component = session.project().unwrap().components().next().unwrap();
        assert_eq!(component.value(), "0.1 Ohm");
        component.display_value().into_owned()
    };
    assert_eq!(shown, "100 mΩ");
    // Submitting the shown text unchanged must not round, re-store or dirty the canonical value.
    apply(&mut session, crate::SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":"R1","value":shown}));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.project().unwrap().components().next().unwrap().value(), "0.1 Ohm");
    let before = session.snapshot().unwrap();
    assert!(
        session
            .apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":symbol,"reference":"R1","value":"1 Ohm","expected_revision":session.revision()}))
            .is_err()
    );
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// A shown value survives undo/redo and folder/archive reopen unchanged; an unset value is not zero.
#[test]
fn displayed_primary_value_survives_history_and_reopen() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    session.execute(sources::LOAD, &json!({"source":"native-parts","documents":documents()})).unwrap();
    let params = placement(&session, false);
    apply(&mut session, PLACE_PART, params);
    let shown = |session: &Session| session.project().unwrap().components().next().unwrap().display_value().into_owned();
    let unset = shown(&session);
    assert!(session.project().unwrap().components().next().unwrap().part.as_ref().unwrap().display_primary_value().is_none());
    assert_ne!(unset, "0 Ω");
    let symbol = session.project().unwrap().symbols()[0].id();
    apply(&mut session, crate::SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":"R1","value":"4.7k"}));
    assert_eq!(shown(&session), "4.7 kΩ");
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    let params = json!({"symbol":symbol,"reference":"R1","value":shown(&session)});
    apply(&mut session, crate::SET_SYMBOL_FIELDS, params);
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    let archive = decode_packed(&encode_packed(session.snapshot().unwrap()).unwrap()).unwrap().project;
    for project in [decode_documents(&saved).unwrap(), archive] {
        let component = project.components().next().unwrap();
        assert_eq!((component.value(), &*component.display_value()), ("4700 Ohm", "4.7 kΩ"));
        assert_eq!(encode_documents(project).unwrap(), saved);
    }
    session.apply("edit.undo", &Json::Null).unwrap();
    assert_eq!(shown(&session), unset);
    session.apply("edit.redo", &Json::Null).unwrap();
    assert_eq!(shown(&session), "4.7 kΩ");
}

/// Every mapped land participates in accepted copper connectivity, including a terminal with no wire.
#[test]
fn multipad_and_nc_packages_are_versioned_and_preserved_offline() {
    let mut documents = documents();
    let mut footprint: Json = serde_json::from_str(&documents["footprints/resistor-0603.json"]).unwrap();
    for (number, x) in [("3", -2_000_000), ("4", 2_000_000)] {
        let mut pad = footprint["pads"][0].clone();
        pad["number"] = json!(number);
        pad["center"] = json!({"x_nm":x,"y_nm":3_000_000});
        footprint["pads"].as_array_mut().unwrap().push(pad);
    }
    documents.insert("footprints/resistor-0603.json".into(), footprint.to_string());
    let mut part: Json = serde_json::from_str(&documents["parts/resistor.json"]).unwrap();
    part["packages"]["0603"]["mapping"] = json!({"1":["1","3"],"2":["2"]});
    part["packages"]["0603"]["unconnected_pads"] = json!(["4"]);
    documents.insert("parts/resistor.json".into(), part.to_string());
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session.execute(sources::LOAD, &json!({"source":"native-parts","documents":documents})).unwrap();
    let params = placement(&session, false);
    apply(&mut session, PLACE_PART, params);
    let component = session.project().unwrap().components().next().unwrap().id();
    apply(&mut session, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":0,"y_nm":0}));
    apply(&mut session, crate::board::update::INITIALIZE_BASELINE, json!({}));
    let project = session.snapshot().unwrap();
    let baseline = project.board.baseline.as_ref().unwrap();
    let definition = project.footprint_catalog().into_iter().find(|d| d.id == "resistor-0603").unwrap();
    assert_eq!(project.default_footprint_mapping(&component, &definition).unwrap(), project.board.footprints()[0].mapping.clone().unwrap());
    assert_eq!(baseline.format, "pcbcraft-board-baseline/2");
    let ratsnest = project.board_view(project.board_target().unwrap()).unwrap().ratsnest().unwrap();
    assert_eq!(ratsnest.net_count, 1);
    assert_eq!(ratsnest.lines.len(), 1, "both lands on a singleton logical terminal still need copper");
    let pads = [&ratsnest.lines[0].from, &ratsnest.lines[0].to]
        .into_iter()
        .map(|endpoint| endpoint.pad.inspect()["pad"].as_str().unwrap().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(pads, std::collections::BTreeSet::from(["1".to_owned(), "3".to_owned()]));
    assert!(ratsnest.missing.is_empty());
    let inspection = project.board.footprints()[0].inspect(&project);
    assert_eq!(inspection["pins"].as_array().unwrap().len(), 4);
    let nc = inspection["pins"].as_array().unwrap().iter().find(|pad| pad["unconnected"] == true).unwrap();
    assert!(nc["terminal"].is_null());
    let selection = crate::board::clipboard::BoardSelection {
        footprints: std::collections::BTreeSet::from([format!("footprint-{}", project.board.footprints()[0].id)]),
        ..Default::default()
    };
    let fragment = crate::board::clipboard::BoardFragment::capture(&project, &selection, false).unwrap();
    let text = fragment.to_text().unwrap();
    assert_eq!(crate::board::clipboard::BoardFragment::from_text(&text).unwrap().to_text().unwrap(), text);
    let docs = encode_documents(project.clone()).unwrap();
    let board_path = docs.iter().find(|(_, bytes)| String::from_utf8_lossy(bytes).contains("pcbcraft-board/17")).unwrap().0;
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    let mut old = docs.clone();
    let text = String::from_utf8(old[board_path].clone()).unwrap().replace("pcbcraft-board/17", "pcbcraft-board/16");
    old.insert(board_path.clone(), text.into_bytes());
    assert!(decode_documents(&old).is_err());
    let mut old_baseline = baseline.as_ref().clone();
    old_baseline.format = "pcbcraft-board-baseline/1".into();
    assert!(old_baseline.validate().is_err());
    apply(&mut session, crate::SET_SYMBOL_FIELDS, json!({"symbol":project.symbols()[0].id(),"reference":"R2","value":"R"}));
    session.apply("edit.undo", &Json::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), docs);
}

/// A dependency refresh invalidates a draft, while missing pinned files reject opening atomically.
#[test]
fn dependency_digests_and_pinned_closure_are_enforced() {
    let mut session = fixture(false);
    let stale = placement(&session, false);
    let mut source_documents = documents();
    let mut category: Json = serde_json::from_str(&source_documents["categories/resistor.json"]).unwrap();
    category["name"] = json!("Changed resistor category");
    source_documents.insert("categories/resistor.json".into(), category.to_string());
    session.execute(sources::LOAD, &json!({"source":"native-parts","documents":source_documents})).unwrap();
    let before = session.snapshot().unwrap();
    session.execute(sources::REMOVE, &json!({"source":"native-parts"})).unwrap();
    let mut source_documents = documents();
    let mut category: Json = serde_json::from_str(&source_documents["categories/resistor.json"]).unwrap();
    category["name"] = json!("Changed resistor category");
    source_documents.insert("categories/resistor.json".into(), category.to_string());
    session.execute(sources::LOAD, &json!({"source":"native-parts","documents":source_documents})).unwrap();
    let mut stale = stale;
    stale["expected_revision"] = json!(session.revision());
    assert!(session.apply(PLACE_PART, &stale).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    let params = placement(&session, true);
    apply(&mut session, PLACE_PART, params);
    let docs = encode_documents(session.snapshot().unwrap()).unwrap();
    for path in docs.keys().filter(|path| path.starts_with("library/")) {
        let mut missing = docs.clone();
        missing.remove(path);
        assert!(decode_documents(&missing).is_err(), "missing {path}");
    }
}

/// Copying a native part transports its complete closure and optional manufacturer selection without a source.
#[test]
fn native_part_clipboard_is_portable() {
    let mut source = fixture(false);
    let params = placement(&source, true);
    apply(&mut source, PLACE_PART, params);
    let symbol = source.project().unwrap().symbols()[0].id();
    let copied = source
        .execute(crate::clipboard::COPY, &json!({"items":{"symbols":[symbol],"wires":[],"net_labels":[]},"expected_revision":source.revision()}))
        .unwrap();
    let mut target = Session::default();
    target.apply("project.new", &json!({"create_board":false})).unwrap();
    let before = target.snapshot().unwrap();
    apply(&mut target, crate::clipboard::PASTE, json!({"text":copied["clipboard"],"x_nm":0,"y_nm":0}));
    let project = target.snapshot().unwrap();
    let part = project.components().next().unwrap().part.as_ref().unwrap();
    assert_eq!(part.manufacturer.as_ref().unwrap().mpn, "TEST-R100M-0603");
    let docs = encode_documents(project).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    target.apply("edit.undo", &Json::Null).unwrap();
    assert!(target.project().unwrap().components().next().is_none());
    assert!(before.components().next().is_none());
}

/// Native selections preserve a legacy flat layout until first-save migration, then retain every pinned record.
#[test]
fn native_part_selection_survives_first_save_storage_migration() {
    for board in [false, true] {
        let mut session = fixture(board);
        // Project the original command-created circuit into the supported historical flat encoding.
        let mut flat = crate::Project::clone(&crate::persistence::diagnostic_snapshot(session.snapshot().unwrap()));
        flat.layout = None;
        let old = encode_documents(Arc::new(flat)).unwrap();
        session.open_snapshot(decode_documents(&old).unwrap());
        let params = placement(&session, true);
        apply(&mut session, PLACE_PART, params);
        let before = session.snapshot().unwrap();
        let docs = encode_documents(before.clone()).unwrap();
        let migrated = crate::persistence::migrate_board_layout(docs.clone(), "electronics").unwrap();
        let reopened = decode_documents(&migrated).unwrap();
        assert_eq!(reopened.layout_root(), Some("electronics"));
        assert_eq!(reopened.id(), before.id());
        assert_eq!(reopened.board_target(), before.board_target());
        assert_eq!(reopened.components().next().unwrap().part, before.components().next().unwrap().part);
        assert_eq!(encode_documents(reopened.clone()).unwrap(), migrated);
        for (path, bytes) in &docs {
            if path != crate::persistence::ENTRY_POINT && path != "board.json" {
                assert_eq!(migrated.get(path), Some(bytes), "migration changed {path}");
            }
        }
        if board {
            assert_eq!(migrated[&reopened.board_document_path().unwrap()], docs["board.json"]);
        }
    }
}

/// Placing a schematic-only native part also upgrades the allocation contract of a pre-watermark project.
#[test]
fn native_part_without_package_adopts_legacy_allocation_watermark() {
    let mut session = fixture(true);
    let mut historical = crate::Project::clone(&crate::persistence::diagnostic_snapshot(session.snapshot().unwrap()));
    historical.layout = None;
    historical.numeric_id_high_water = None;
    Arc::make_mut(&mut historical.board).ownership = crate::board::identity::Ownership::Legacy;
    let documents = encode_documents(Arc::new(historical)).unwrap();
    session = Session { libraries: session.libraries.clone(), ..Default::default() };
    session.open_snapshot(decode_documents(&documents).unwrap());
    assert!(session.project().unwrap().numeric_id_high_water.is_none());
    let mut params = placement(&session, false);
    params["package"] = Json::Null;
    apply(&mut session, PLACE_PART, params);
    let project = session.snapshot().unwrap();
    assert!(project.numeric_id_high_water.is_some());
    assert!(project.footprint_assignments().is_empty());
    let documents = encode_documents(project).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
}

/// Re-registering different dependency content under the same revision cannot retarget an existing part through MPN selection.
#[test]
fn manufacturer_selection_rejects_conflicting_pinned_dependencies() {
    let mut session = fixture(true);
    let params = placement(&session, false);
    apply(&mut session, PLACE_PART, params);
    let before = session.snapshot().unwrap();
    let component = before.components().next().unwrap().id();
    session.execute(sources::REMOVE, &json!({"source":"native-parts"})).unwrap();
    let mut documents = documents();
    let mut part: Json = serde_json::from_str(&documents["parts/resistor.json"]).unwrap();
    part["fixed_parameters"] = json!({"pcbcraft:resistance":{"type":"quantity","value":"0.1","unit":"Ohm"}});
    documents.insert("parts/resistor.json".into(), part.to_string());
    let mut footprint: Json = serde_json::from_str(&documents["footprints/resistor-0603.json"]).unwrap();
    footprint["pads"][0]["width_nm"] = json!(760_000);
    documents.insert("footprints/resistor-0603.json".into(), footprint.to_string());
    session.execute(sources::LOAD, &json!({"source":"native-parts","documents":documents})).unwrap();
    let manufacturer =
        session.libraries.manufacturers_for(&Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 }).pop().unwrap();
    let result = session.apply(
        SET_PART,
        &json!({"component":component,"manufacturer":manufacturer.reference(),
        "manufacturer_digest":sources::manufacturer_digest(&manufacturer),"expected_revision":session.revision()}),
    );
    assert!(result.unwrap_err().to_string().contains("pinned part revision"));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}
