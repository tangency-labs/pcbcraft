use super::*;
use crate::{
    PLACE_FOOTPRINT, PLACE_SYMBOL, SET_FOOTPRINT_TRANSFORM, Session,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::{Value, json};

/// Build through public commands with a real, addressed two-layer board.
fn session() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true,})).unwrap();
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    s
}
/// Place a symbol and return its inspected identity without assuming allocator positions.
fn symbol(s: &mut Session, name: &str) -> String {
    s.apply(PLACE_SYMBOL, &json!({"definition":name,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    s.project().unwrap().symbols().last().unwrap().id()
}
/// Assign a built-in definition by explicit terminal-to-pad identity and retain its placement ID.
fn assign(s: &mut Session, symbol: &str, definition: &str, mapping: Value) -> String {
    s.apply(PLACE_FOOTPRINT, &json!({"symbol":symbol,"definition":definition,"mapping":mapping,"x_nm":0,"y_nm":0,"expected_revision":s.revision()}))
        .unwrap();
    s.inspect()["project"]["board"]["footprints"].as_array().unwrap().last().unwrap()["id"].as_str().unwrap().into()
}

/// Library defaults and board adjustments compose before clipping, while authoring preserves pads.
#[test]
fn library_and_board_reference_offsets_are_additive() {
    use crate::authoring::{Action, FootprintReferenceSettings, Item};
    let original = starter("resistor-0402").unwrap();
    let mut editor = crate::authoring::Session::new(Item::from_footprint(&original).unwrap(), false).unwrap();
    let before = editor.snapshot();
    let default = Point { x_nm: 2_000_000, y_nm: -1_000_000 };
    editor
        .execute(
            editor.revision(),
            Action::Metadata {
                name: before.name.clone(),
                license: before.license.clone(),
                source: before.source.clone(),
                reference_prefix: before.reference_prefix.clone(),
                default_value: before.default_value.clone(),
                symbol_settings: None,
                footprint_reference: Some(FootprintReferenceSettings { enabled: true, offset: default }),
            },
        )
        .unwrap();
    let changed = editor.snapshot().footprint().unwrap();
    assert_eq!(changed.pads, original.pads);
    assert_eq!(changed.graphics, original.graphics);
    assert_eq!(changed.silkscreen, original.silkscreen);
    let offset = Point { x_nm: -1_000_000, y_nm: -2_000_000 };
    let baseline = original.reference_paths("R12", Default::default(), super::super::DEFAULT_REFERENCE_OFFSET);
    let expected: Vec<Vec<Point>> = baseline
        .iter()
        .map(|path| path.iter().map(|p| Point { x_nm: p.x_nm + default.x_nm + offset.x_nm, y_nm: p.y_nm + default.y_nm + offset.y_nm }).collect())
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(changed.reference_paths("R12", Default::default(), offset), expected);
    let exported = editor.snapshot().export_native().unwrap();
    let parsed = read_saved(&serde_json::to_vec(&exported).unwrap()).unwrap();
    assert_eq!(parsed.reference_offset, Some(default));
    assert_eq!(parsed.reference_paths("R12", Default::default(), offset), expected);
}

/// All original definitions validate; rectangular geometry and a three-pin bijection survive
/// rotation, history and offline folder/archive decoding without changing schematic intent.
#[test]
fn smd_geometry_and_three_terminal_mappings_are_pinned_and_persistent() {
    for id in STARTER_IDS {
        starter(id).unwrap().validate().unwrap();
    }
    let mut s = session();
    let r = symbol(&mut s, "resistor");
    let rfp = assign(&mut s, &r, "resistor-0402", json!({"1":"1","2":"2"}));
    let led = symbol(&mut s, "led");
    assign(&mut s, &led, "led-0402", json!({"1":"2","2":"1"}));
    let q = symbol(&mut s, "npn");
    assign(&mut s, &q, "sot23", json!({"1":"1","2":"2","3":"3"}));
    let header = symbol(&mut s, "connector");
    assign(&mut s, &header, "header-1x02-254", json!({"1":"1","2":"2"}));
    let snapshot = s.snapshot().unwrap();
    let board = snapshot.board_view(snapshot.board_target().unwrap()).unwrap();
    let resistor = board.footprints().find(|f| f.id == rfp).unwrap();
    assert_eq!(resistor.pads.len(), 2);
    let copper = resistor.pads[0].copper.unwrap();
    assert_eq!(copper[1].x_nm - copper[0].x_nm, 600_000);
    assert_eq!(copper[2].y_nm - copper[1].y_nm, 600_000);
    assert_eq!(resistor.pads[0].center.x_nm, -525_000);
    assert_eq!(resistor.pads[0].shape, PadShape::RoundedRectangle);
    assert_eq!(resistor.pads[0].corner_radius_nm, 150_000);
    assert_eq!(resistor.pads[0].drill_nm, None);
    let header = board.footprints().find(|f| f.definition == "header-1x02-254").unwrap();
    assert_eq!(header.pads[0].shape, PadShape::Rectangle);
    assert_eq!(header.pads[1].shape, PadShape::Circle);
    assert!(header.pads.iter().all(|pad| pad.drill_nm == Some(1_000_000) && pad.through_hole));
    let transistor = board.footprints().find(|f| f.component == snapshot.component(&snapshot.symbols()[2]).unwrap().id()).unwrap();
    assert_eq!(transistor.pads.len(), 3);
    let cathode = crate::schematic::Terminal::parse(&json!({"symbol":led,"terminal":"2"})).unwrap();
    assert_eq!(snapshot.pad_for_terminal(cathode).unwrap().inspect()["pad"], "1");
    for pin in snapshot.symbols().iter().flat_map(|s| s.terminals()) {
        let pad = snapshot.pad_for_terminal(pin).unwrap();
        assert_eq!(snapshot.terminal_for_pad(pad), Some(pin));
    }
    let bytes = encode_documents(snapshot.clone()).unwrap();
    assert!(bytes.contains_key("library/footprints/starter/resistor-0402.r4.json"));
    assert!(bytes.contains_key("library/footprints/starter/sot23.r4.json"));
    assert!(std::str::from_utf8(&bytes[&crate::persistence::test_board_path(&bytes)]).unwrap().contains("pcbcraft-board/5"));
    assert_eq!(encode_documents(decode_documents(&bytes).unwrap()).unwrap(), bytes);
    assert_eq!(encode_documents(decode_packed(&encode_packed(snapshot.clone()).unwrap()).unwrap().project).unwrap(), bytes);
    let schematic = s.inspect()["project"]["schematic"].clone();
    s.apply(SET_FOOTPRINT_TRANSFORM, &json!({"footprint":rfp,"x_nm":2_000_000,"y_nm":1_000_000,"rotation":90,"expected_revision":s.revision()}))
        .unwrap();
    let changed = s.snapshot().unwrap();
    let rotated = changed.board_view(changed.board_target().unwrap()).unwrap().footprints().find(|f| f.id == rfp).unwrap();
    assert_eq!(rotated.pads[0].center, Point { x_nm: 2_000_000, y_nm: 1_525_000 });
    assert_eq!(s.inspect()["project"]["schematic"], schematic);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
}

/// An unavailable custom library still reopens from its exact saved copy. No built-in ID or
/// installed source is needed to resolve pad dimensions, graphics or component correspondence.
#[test]
fn pinned_custom_footprints_reopen_without_a_catalog_source() {
    let mut s = session();
    let r = symbol(&mut s, "resistor");
    assign(&mut s, &r, "resistor-0603", json!({"1":"1","2":"2"}));
    let mut docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let original = "library/footprints/starter/resistor-0603.r4.json";
    let mut definition: Value = serde_json::from_slice(&docs.remove(original).unwrap()).unwrap();
    definition["library"] = json!("offline-fixture");
    definition["id"] = json!("custom-resistor");
    definition["pads"][0]["width_nm"] = json!(900_000);
    docs.insert("library/footprints/offline-fixture/custom-resistor.r4.json".into(), serde_json::to_vec_pretty(&definition).unwrap());
    let mut board: Value = serde_json::from_slice(&docs[&crate::persistence::test_board_path(&docs)]).unwrap();
    board["footprints"][0]["library"] = json!("offline-fixture");
    board["footprints"][0]["definition"] = json!("custom-resistor");
    docs.insert(crate::persistence::test_board_path(&docs), serde_json::to_vec_pretty(&board).unwrap());
    let reopened = decode_documents(&docs).unwrap();
    let footprint = reopened.board_view(reopened.board_target().unwrap()).unwrap().footprints().next().unwrap();
    let copper = footprint.pads[0].copper.unwrap();
    assert_eq!(copper[1].x_nm - copper[0].x_nm, 900_000);
    let canonical = encode_documents(reopened.clone()).unwrap();
    assert_eq!(encode_documents(decode_documents(&canonical).unwrap()).unwrap(), canonical);
    assert!(starter("custom-resistor").is_none());
}

/// Invalid pad sets, malformed geometry, missing definitions and mismatched assignments
/// are refused without partial pinning, placement, history or coordinate changes.
#[test]
fn invalid_physical_footprints_and_mappings_are_atomic() {
    let mut s = session();
    let q = symbol(&mut s, "npn");
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    for mapping in [json!({"1":"1","2":"2"}), json!({"1":"1","2":"1","3":"3"}), json!({"1":"1","2":"2","4":"3"})] {
        assert!(
            s.apply(PLACE_FOOTPRINT, &json!({"symbol":q,"definition":"sot23","mapping":mapping,"x_nm":0,"y_nm":0,"expected_revision":revision}))
                .is_err()
        );
        assert_eq!(s.revision(), revision);
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    assign(&mut s, &q, "sot23", json!({"1":"1","2":"2","3":"3"}));
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = "library/footprints/starter/sot23.r4.json";
    for (pointer, value) in [
        ("/pads/0/width_nm", json!(0)),
        ("/pads/1/number", json!("1")),
        ("/pads/0/center/x_nm", json!(i64::MIN)),
        ("/pads/0/corner_radius_nm", json!(-1)),
        ("/pads/0/corner_radius_nm", json!(700_000)),
        ("/pads/0/drill_nm", json!(100_000)),
        ("/pads/0/shape", json!("circle")),
        ("/format", json!("pcbcraft-footprint/2")),
    ] {
        let mut bad = docs.clone();
        let mut d: Value = serde_json::from_slice(&bad[path]).unwrap();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        d.pointer_mut(parent).unwrap().as_object_mut().unwrap().insert(field.into(), value);
        bad.insert(path.into(), serde_json::to_vec(&d).unwrap());
        assert!(decode_documents(&bad).is_err());
    }
    let mut missing = docs.clone();
    missing.remove(path);
    assert!(decode_documents(&missing).is_err());
    let mut old = docs.clone();
    let mut b: Value = serde_json::from_slice(&old[&crate::persistence::test_board_path(&old)]).unwrap();
    b["format"] = json!("pcbcraft-board/4");
    old.insert(crate::persistence::test_board_path(&old), serde_json::to_vec(&b).unwrap());
    assert!(decode_documents(&old).unwrap_err().contains("format 5"));
    assert!(crate::persistence::encode(s.snapshot().unwrap()).is_err());
}

/// Drill ownership and annulus dimensions are validated before a pinned project is adopted.
#[test]
fn plated_drills_require_all_copper_and_a_positive_annulus() {
    let mut s = session();
    let header = symbol(&mut s, "connector");
    assign(&mut s, &header, "header-1x02-254", json!({"1":"1","2":"2"}));
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let path = "library/footprints/starter/header-1x02-254.r3.json";
    for (field, value) in [
        ("drill_nm", json!(0)),
        ("drill_nm", json!(-1)),
        ("drill_nm", json!(1_700_000)),
        ("drill_nm", json!(null)),
        ("copper", json!("component_side")),
    ] {
        let mut bad = docs.clone();
        let mut definition: Value = serde_json::from_slice(&bad[path]).unwrap();
        definition["pads"][0][field] = value;
        bad.insert(path.into(), serde_json::to_vec(&definition).unwrap());
        assert!(decode_documents(&bad).is_err());
    }
}

/// Physical mappings, per-sheet net annotations and pinned geometry survive a multi-sheet
/// folder/archive round trip and resolve independently of the requesting schematic projection.
#[test]
fn physical_footprints_on_multiple_sheets_share_the_board_without_aliasing_nets() {
    let mut s = session();
    s.apply(crate::sheets::CREATE_SHEET, &json!({"name":"Aux","expected_revision":s.revision()})).unwrap();
    let sheets: Vec<_> = s.project().unwrap().sheets().map(|sheet| (sheet.target, sheet.name.to_owned())).collect();
    for (target, _) in &sheets {
        let mut symbols = Vec::new();
        for x in [-10_000_000, 10_000_000] {
            s.apply(PLACE_SYMBOL, &json!({"sheet":target.sheet,"definition":"resistor","x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
            symbols.push(s.project().unwrap().for_sheet(*target).unwrap().symbols().last().unwrap().id());
        }
        s.apply(crate::CONNECT_PINS,&json!({"sheet":target.sheet,"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"},"expected_revision":s.revision()})).unwrap();
        s.apply(PLACE_FOOTPRINT,&json!({"sheet":target.sheet,"symbol":symbols[0],"definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    let snapshot = s.snapshot().unwrap();
    let target = snapshot.board_target().unwrap();
    for (sheet, _) in &sheets {
        let projected = snapshot.for_sheet(*sheet).unwrap();
        let footprints: Vec<_> = projected.board_view(target).unwrap().footprints().collect();
        assert_eq!(footprints.len(), 2);
        for (footprint, (_, name)) in footprints.iter().zip(&sheets) {
            assert_eq!(footprint.pads.iter().find(|pad| pad.number == "2").unwrap().net, Some(format!("{name}: Net 1")));
            assert_eq!(footprint.pads.iter().find(|pad| pad.number == "1").unwrap().net, None);
        }
    }
    for symbol in snapshot.all_symbols() {
        for terminal in symbol.terminals() {
            if let Some(pad) = snapshot.pad_for_terminal(terminal) {
                assert_eq!(snapshot.terminal_for_pad(pad), Some(terminal));
            }
        }
    }
    let docs = encode_documents(snapshot.clone()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    assert_eq!(encode_documents(decode_packed(&encode_packed(snapshot).unwrap()).unwrap().project).unwrap(), docs);
}
