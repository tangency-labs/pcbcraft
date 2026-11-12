use super::*;
use crate::schematic::GRID_NM as G;

mod drag;

/// Exercise catalog validation and revision checks for every authored test operation.
fn apply(s: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(s.revision());
    s.apply(command, &params).unwrap();
}
/// Create an isolated in-memory project through the production command path.
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s
}
/// Place a real starter resistor and return its explicit terminal position.
fn resistor(s: &mut Session, sheet: SheetTarget, x: i64) -> Terminal {
    apply(s, PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":x,"y_nm":0}));
    s.project().unwrap().for_sheet(sheet).unwrap().symbols().last().unwrap().terminals()[0]
}
/// Attach an authored name at a pin using the same exact anchor offered in the GUI.
fn label(s: &mut Session, sheet: SheetTarget, pin: Terminal, name: &str, style: &str) -> String {
    let at = s.project().unwrap().for_sheet(sheet).unwrap().terminal_position(pin).unwrap();
    apply(s, ADD, json!({"sheet":sheet.id(),"name":name,"style":style,"x_nm":at.x_nm,"y_nm":at.y_nm}));
    s.project().unwrap().for_sheet(sheet).unwrap().net_labels().last().unwrap().object_id()
}

/// Local names merge only on their own sheet; global names bridge sheets and also
/// join same-name locals within a sheet. Case and rail names remain electrically distinct.
#[test]
fn scope_and_case_are_resolved_before_sheet_projection() {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, sheets::CREATE_SHEET, json!({"name":"Child"}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    let a = resistor(&mut s, main, 0);
    let b = resistor(&mut s, main, 20 * G);
    let c = resistor(&mut s, child, 0);
    label(&mut s, main, a, "GND", "local");
    label(&mut s, main, b, "GND", "local");
    let remote = label(&mut s, child, c, "GND", "global");
    assert_eq!(s.project().unwrap().electrical_connectivity().net_for(a).unwrap(), &[a, b]);
    let first = s.project().unwrap().for_sheet(main).unwrap().net_labels()[0].clone();
    apply(&mut s, SET, json!({"sheet":main.id(),"id":first.object_id(),"name":"GND","style":"global","x_nm":first.at.x_nm,"y_nm":first.at.y_nm}));
    let p = s.project().unwrap();
    assert_eq!(p.electrical_connectivity().net_for(a).unwrap(), &[a, b, c]);
    let local = p.for_sheet(child).unwrap();
    assert_eq!(local.connectivity().net_for(c).unwrap(), &[c]);
    assert_eq!(local.connectivity().net_name(0), "GND");
    let at = local.net_labels()[0].at;
    apply(&mut s, SET, json!({"sheet":child.id(),"id":remote,"name":"gnd","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm}));
    assert_eq!(s.project().unwrap().electrical_connectivity().net_for(a).unwrap(), &[a, b]);
    apply(&mut s, SET, json!({"sheet":child.id(),"id":remote,"name":"+3V3","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm}));
    assert_eq!(s.project().unwrap().electrical_connectivity().net_for(c).unwrap(), &[c]);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.project().unwrap().for_sheet(child).unwrap().net_labels()[0].name, "gnd");
}

/// Native arrow and bar symbols join by their declared terminal role across sheets, and
/// pinning retains their distinct drawings without any power-marker annotation records.
#[test]
fn native_power_drawings_share_global_values_across_sheets() {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, sheets::CREATE_SHEET, json!({"name":"Supply"}));
    let other = s.project().unwrap().sheets().nth(1).unwrap().target;
    let a = resistor(&mut s, main, 0);
    let b = resistor(&mut s, other, 0);
    for (sheet, pin, id) in [(main, a, "vcc"), (other, b, "vcc-bar")] {
        let at = s.project().unwrap().for_sheet(sheet).unwrap().terminal_position(pin).unwrap();
        let d = s.libraries().latest_symbol("power", id).unwrap();
        apply(
            &mut s,
            PLACE_SYMBOL,
            json!({"sheet":sheet.id(),"library":"power","definition":id,"definition_revision":d.revision(),"definition_digest":library::sources::symbol_digest(&d).unwrap(),"x_nm":at.x_nm,"y_nm":at.y_nm}),
        );
    }
    let p = s.snapshot().unwrap();
    let g = p.electrical_connectivity();
    assert_eq!(g.net_index_for(a), g.net_index_for(b));
    assert_eq!(g.net_name(g.net_index_for(a).unwrap()), "VCC");
    assert!(p.sheets().all(|sheet| p.for_sheet(sheet.target).unwrap().net_labels().is_empty()));
    let docs = persistence::encode_documents(p).unwrap();
    let reopened = persistence::decode_documents(&docs).unwrap();
    assert_eq!(reopened.for_sheet(main).unwrap().symbols()[1].definition.id, "vcc");
    assert_eq!(reopened.for_sheet(other).unwrap().symbols()[1].definition.id, "vcc-bar");
    assert_eq!(persistence::encode_documents(reopened).unwrap(), docs);
}

/// A label at a wire interior names that explicit route; ordinary geometric crossings remain
/// separate until a label is deliberately placed there. Geometry edits invalidate contact caches.
#[test]
fn wire_contacts_and_moving_pins_rebuild_named_connectivity() {
    let mut s = session();
    let sheet = s.project().unwrap().sheet_target();
    let a = resistor(&mut s, sheet, 0);
    let b = resistor(&mut s, sheet, 20 * G);
    label(&mut s, sheet, a, "VCC", "global");
    label(&mut s, sheet, b, "VCC", "global");
    let old = s.snapshot().unwrap();
    assert_eq!(old.connectivity().net_for(a).unwrap(), &[a, b]);
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":a.inspect()["symbol"],"x_nm":0,"y_nm":4*G}));
    assert!(s.project().unwrap().connectivity().net_for(a).is_none());
    assert_eq!(old.connectivity().net_for(a).unwrap(), &[a, b]);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.project().unwrap().connectivity().net_for(a).unwrap(), &[a, b]);
    for (from, to) in [((0, 10 * G), (20 * G, 10 * G)), ((10 * G, 5 * G), (10 * G, 15 * G))] {
        apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":from.0,"y_nm":from.1},"to":{"x_nm":to.0,"y_nm":to.1},"bends":[]}));
    }
    let n = s.project().unwrap().connectivity().nets.len();
    apply(&mut s, ADD, json!({"name":"BUSY","style":"local","x_nm":5*G,"y_nm":10*G}));
    assert_eq!(s.project().unwrap().connectivity().nets.len(), n);
    let p = s.project().unwrap();
    let wire = &p.wires()[p.wires().len() - 2];
    let index = p.connectivity().net_index_for_endpoint(wire.endpoints()[0]).unwrap();
    assert_eq!(p.connectivity().net_name(index), "BUSY");
    apply(&mut s, ADD, json!({"name":"JOIN","style":"local","x_nm":10*G,"y_nm":10*G}));
    assert_eq!(s.project().unwrap().connectivity().nets.len(), n - 1);
}

/// Format 3 is required only for sheets with named anchors. Allocation changes the entry-point
/// watermark and the owning sheet; other documents stay identical and archives round-trip.
#[test]
fn storage_roundtrip_and_small_diffs_preserve_old_documents() {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, sheets::CREATE_SHEET, json!({"name":"Second"}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    let before = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    apply(&mut s, ADD, json!({"sheet":main.id(),"name":"GND","style":"global","x_nm":0,"y_nm":0,"rotation":90}));
    let docs = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let path = format!("sheets/{}.json", main.id());
    assert_eq!(docs.iter().filter(|(k, v)| before.get(*k) != Some(*v)).map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["project.json", &path]);
    assert!(String::from_utf8(docs[&path].clone()).unwrap().contains("pcbcraft-sheet/3"));
    let reopened = persistence::decode_documents(&docs).unwrap();
    assert_eq!(persistence::encode_documents(reopened.clone()).unwrap(), docs);
    let packed = persistence::encode_packed(reopened.clone()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_packed(&packed).unwrap().project).unwrap(), docs);
    assert!(persistence::encode(reopened).is_err());
    let label = s.project().unwrap().net_labels()[0].clone();
    apply(&mut s, SET, json!({"sheet":main.id(),"id":label.object_id(),"name":"GND","style":"global","x_nm":G,"y_nm":0,"rotation":180}));
    let moved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let old = String::from_utf8(docs[&path].clone()).unwrap();
    let new = String::from_utf8(moved[&path].clone()).unwrap();
    assert_eq!(old.lines().zip(new.lines()).filter(|(a, b)| a != b).count(), 1);
    assert_eq!(docs[&format!("sheets/{}.json", child.id())], moved[&format!("sheets/{}.json", child.id())]);
}

/// Invalid text/rotation/foreign IDs/stale revisions never replace the snapshot. No-op edits
/// retain revision, and malformed persisted names/IDs/versions/endpoints fail before adoption.
#[test]
fn invalid_edits_and_documents_are_atomic() {
    let mut s = session();
    apply(&mut s, ADD, json!({"name":"GND","style":"global","x_nm":0,"y_nm":0}));
    let label = s.project().unwrap().net_labels()[0].clone();
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let good = json!({"id":label.object_id(),"name":"GND","style":"global","x_nm":0,"y_nm":0,"expected_revision":revision});
    s.apply(SET, &good).unwrap();
    assert_eq!(s.revision(), revision);
    for patch in [
        json!({"name":""}),
        json!({"name":" GND"}),
        json!({"name":"GND\n"}),
        json!({"rotation":45}),
        json!({"style":"unknown"}),
        json!({"id":"net-label-9999"}),
        json!({"expected_revision":0}),
    ] {
        let mut params = good.clone();
        params.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        assert!(s.apply(SET, &params).is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let docs = persistence::encode_documents(before).unwrap();
    let path = format!("sheets/{}.json", label_sheet(&s));
    for (key, value) in [("name", json!("")), ("rotation", json!(45)), ("id", json!(0)), ("style", json!("unknown"))] {
        let mut bad = docs.clone();
        let mut sheet: Value = serde_json::from_slice(&bad[&path]).unwrap();
        sheet["net_labels"][0][key] = value;
        bad.insert(path.clone(), serde_json::to_vec(&sheet).unwrap());
        assert!(persistence::decode_documents(&bad).is_err());
    }
    for version in ["pcbcraft-sheet/1", "pcbcraft-sheet/2"] {
        let mut bad = docs.clone();
        let mut sheet: Value = serde_json::from_slice(&bad[&path]).unwrap();
        sheet["format"] = json!(version);
        bad.insert(path.clone(), serde_json::to_vec(&sheet).unwrap());
        assert!(persistence::decode_documents(&bad).is_err());
    }
}
/// Read the explicitly inspected sheet identity for codec mutation tests.
fn label_sheet(s: &Session) -> uuid::Uuid {
    s.project().unwrap().sheet_target().id()
}

/// Wires can directly address a named anchor, and deleting it cleans only its owned attachment.
#[test]
fn direct_endpoint_draw_delete_and_history() {
    let mut s = session();
    apply(&mut s, ADD, json!({"name":"+3V3","style":"global","x_nm":0,"y_nm":0}));
    let label = s.project().unwrap().net_labels()[0].clone();
    apply(&mut s, DRAW_WIRE, json!({"from":{"net_label":label.object_id()},"to":{"x_nm":10*G,"y_nm":0},"bends":[]}));
    let before = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    apply(&mut s, DELETE, json!({"id":label.object_id()}));
    assert!(s.project().unwrap().wires().is_empty());
    assert!(s.project().unwrap().net_labels().is_empty());
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert!(s.project().unwrap().net_labels().is_empty());
}

/// Net-name appearance changes neither the electrical anchor nor attached routes, and is
/// preserved by later edits that omit text. Older sheet versions cannot discard it on load.
#[test]
fn label_text_appearance_is_persistent_atomic_and_separate_from_wiring() {
    let mut s = session();
    apply(&mut s, ADD, json!({"name":"GND","style":"global","x_nm":0,"y_nm":0}));
    let label = s.project().unwrap().net_labels()[0].clone();
    apply(&mut s, DRAW_WIRE, json!({"from":{"net_label":label.object_id()},"to":{"x_nm":10*G,"y_nm":0},"bends":[]}));
    let before = s.snapshot().unwrap();
    let text = schematic::fields::Layout { offset: Some(Point { x_nm: 2 * G, y_nm: 4 * G }), height_nm: 2_000_000, ..Default::default() };
    apply(&mut s, SET, json!({"id":label.object_id(),"name":"GND","style":"global","x_nm":0,"y_nm":0,"text":text}));
    let after = s.snapshot().unwrap();
    assert_eq!(before.inspect_schematic()["netlist"], after.inspect_schematic()["netlist"]);
    assert_eq!(before.wire_path(&before.wires()[0]), after.wire_path(&after.wires()[0]));
    assert_eq!(after.net_labels()[0].at, label.at);
    let documents = persistence::encode_documents(after.clone()).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
    let path = format!("sheets/{}.json", after.sheet_target().id());
    let mut downgraded = documents.clone();
    let mut sheet: Value = serde_json::from_slice(&downgraded[&path]).unwrap();
    assert_eq!(sheet["format"], "pcbcraft-sheet/5");
    sheet["format"] = json!("pcbcraft-sheet/3");
    downgraded.insert(path, serde_json::to_vec(&sheet).unwrap());
    assert!(persistence::decode_documents(&downgraded).is_err());
    for invalid in [json!(null), json!({"height_nm":-1}), json!(schematic::fields::Layout { height_nm: -1, ..Default::default() })] {
        let revision = s.revision();
        assert!(
            s.apply(
                SET,
                &json!({"id":label.object_id(),"name":"BAD","style":"global","x_nm":G,"y_nm":G,"text":invalid,"expected_revision":revision})
            )
            .is_err()
        );
        assert!(Arc::ptr_eq(&after, &s.snapshot().unwrap()));
    }
    apply(&mut s, SET, json!({"id":label.object_id(),"name":"GND","style":"local","x_nm":0,"y_nm":0}));
    assert_eq!(s.project().unwrap().net_labels()[0].text, Some(text));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&after, &s.snapshot().unwrap()));
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// A remote wire can bridge two differently named global nets; local inspection must see the
/// resulting connection too, while retaining both names for inspection and future ERC.
#[test]
fn remote_alias_bridge_merges_local_groups_and_preserves_names() {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, sheets::CREATE_SHEET, json!({"name":"Bridge"}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    let a = resistor(&mut s, main, 0);
    let b = resistor(&mut s, main, 20 * G);
    label(&mut s, main, a, "VCC", "global");
    label(&mut s, main, b, "+3V3", "global");
    for (name, x) in [("VCC", 0), ("+3V3", 10 * G)] {
        apply(&mut s, ADD, json!({"sheet":child.id(),"name":name,"style":"global","x_nm":x,"y_nm":0}));
    }
    let labels = s.project().unwrap().for_sheet(child).unwrap().net_labels().to_vec();
    let old = s.snapshot().unwrap();
    assert_ne!(old.connectivity().net_index_for(a), old.connectivity().net_index_for(b));
    apply(
        &mut s,
        DRAW_WIRE,
        json!({"sheet":child.id(),"from":{"net_label":labels[0].object_id()},"to":{"net_label":labels[1].object_id()},"bends":[]}),
    );
    let graph = s.project().unwrap().connectivity();
    assert_eq!(graph.net_for(a).unwrap(), &[a, b]);
    assert_eq!(graph.net_names(graph.net_index_for(a).unwrap()), &["+3V3", "VCC"]);
    let doc = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(persistence::decode_documents(&doc).unwrap().connectivity().net_for(a).unwrap(), &[a, b]);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&old, &s.snapshot().unwrap());
}

/// Named labels share hierarchical interfaces by name only within that interface's own sheet.
/// Their explicit parent/child pin link still carries that connection across the hierarchy.
#[test]
fn names_work_with_hierarchical_interfaces() {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    apply(&mut s, hierarchy::CREATE_CHILD, json!({"name":"Child","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, hierarchy::ADD_LABEL, json!({"sheet":child.id(),"name":"ENABLE","direction":"input","x_nm":0,"y_nm":0}));
    let a = resistor(&mut s, child, 20 * G);
    label(&mut s, child, a, "ENABLE", "local");
    let interface = s.project().unwrap().for_sheet(child).unwrap().hierarchical_labels()[0].id;
    apply(&mut s, DRAW_WIRE, json!({"sheet":main.id(),"from":{"sheet_pin":format!("label-{interface}")},"to":{"x_nm":-10*G,"y_nm":4*G},"bends":[]}));
    let graph = s.project().unwrap().electrical_connectivity();
    assert_eq!(graph.net_index_for_endpoint(WireEndpoint::SheetPin(interface)), graph.net_index_for(a));
    let docs = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(persistence::decode_documents(&docs).is_ok());
}

/// Codecs reject missing/null arrays, cross-sheet endpoints and object-ID collisions before
/// adoption; legal label anchors remain connectable after a save/reopen.
#[test]
fn codec_rejects_missing_arrays_foreign_endpoints_and_colliding_ids() {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    let a = resistor(&mut s, main, 0);
    let id = label(&mut s, main, a, "GND", "global");
    apply(&mut s, DRAW_WIRE, json!({"from":{"net_label":id},"to":{"x_nm":-10*G,"y_nm":0},"bends":[]}));
    apply(&mut s, sheets::CREATE_SHEET, json!({"name":"Other"}));
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, ADD, json!({"sheet":child.id(),"name":"VCC","style":"global","x_nm":0,"y_nm":0}));
    let remote = s.project().unwrap().for_sheet(child).unwrap().net_labels()[0].id;
    let docs = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let path = format!("sheets/{}.json", main.id());
    for mutation in 0..5 {
        let mut bad = docs.clone();
        let mut doc: Value = serde_json::from_slice(&bad[&path]).unwrap();
        match mutation {
            0 => {
                doc.as_object_mut().unwrap().remove("net_labels");
            }
            1 => doc["net_labels"] = Value::Null,
            2 => doc["wires"][0]["from"] = json!({"net_label":remote}),
            3 => doc["net_labels"][0]["id"] = doc["symbols"][0]["id"].clone(),
            _ => doc["net_labels"][0]["unexpected"] = json!(true),
        }
        bad.insert(path.clone(), serde_json::to_vec(&doc).unwrap());
        assert!(persistence::decode_documents(&bad).is_err(), "mutation {mutation}");
    }
}

/// Marker previews preserve wire identity and move the displayed endpoint without publishing
/// state. Invalid previews fail while the original snapshot and cached graph remain intact.
#[test]
fn marker_preview_moves_attached_wire_without_committing() {
    let mut s = session();
    apply(&mut s, ADD, json!({"name":"GND","style":"global","x_nm":0,"y_nm":0}));
    let label = s.project().unwrap().net_labels()[0].clone();
    apply(&mut s, DRAW_WIRE, json!({"from":{"net_label":label.object_id()},"to":{"x_nm":10*G,"y_nm":0},"bends":[]}));
    let p = s.snapshot().unwrap();
    let at = Point { x_nm: 0, y_nm: 5 * G };
    let preview = p.preview_net_label(label.id, at, 90).unwrap();
    assert_eq!(preview.wire_path(&preview.wires()[0])[0], at);
    assert_eq!(p.net_labels()[0].at, label.at);
    assert_eq!(p.wires()[0].id(), preview.wires()[0].id());
    assert!(p.preview_net_label(label.id, at, 45).is_err());
    assert!(Arc::ptr_eq(&p, &s.snapshot().unwrap()));
}

/// A label touching a deliberately retained loop connects even when that wire passes its
/// anchor more than once. Electrical contact does not need the unique index required by Split.
#[test]
fn labels_contact_repeated_points_on_unhealed_routes() {
    let mut s = session();
    apply(
        &mut s,
        DRAW_WIRE,
        json!({"from":{"x_nm":-10*G,"y_nm":0},"to":{"x_nm":10*G,"y_nm":0},"bends":[{"x_nm":-5*G,"y_nm":0},{"x_nm":-5*G,"y_nm":5*G},{"x_nm":5*G,"y_nm":5*G},{"x_nm":5*G,"y_nm":0},{"x_nm":-5*G,"y_nm":0},{"x_nm":-5*G,"y_nm":-5*G},{"x_nm":10*G,"y_nm":-5*G}],"auto_heal":false}),
    );
    apply(&mut s, ADD, json!({"name":"LOOP","style":"local","x_nm":-5*G,"y_nm":0}));
    let graph = s.project().unwrap().connectivity();
    assert_eq!(graph.nets.len(), 1);
    assert_eq!(graph.net_name(0), "LOOP");
}

/// Removed marker spellings are rejected by the public command and strict native decoder,
/// so no caller can create an invisible or silently substituted power annotation.
#[test]
fn obsolete_power_marker_styles_are_refused_atomically() {
    let mut s = session();
    let before = s.snapshot().unwrap();
    for style in ["power", "power_bar", "ground"] {
        assert!(s.apply(ADD, &json!({"name":"RAIL","style":style,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert!(serde_json::from_value::<Label>(json!({"id":1,"name":"RAIL","style":style,"at":{"x_nm":0,"y_nm":0},"rotation":0})).is_err());
    }
    assert_eq!(crate::command(ADD).unwrap().to_json()["params"]["properties"]["style"]["enum"], json!(["local", "global"]));
}
