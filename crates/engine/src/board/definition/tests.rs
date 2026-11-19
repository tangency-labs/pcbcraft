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

/// Isolate footprint codec boundaries from the editor and project integration.
#[test]
fn circular_npth_native_codec_is_lazy_and_closed() {
    let mut definition = (*basic_components("resistor-0402").unwrap()).clone();
    let legacy = write_saved(&definition).unwrap();
    let old_writer = serde_json::to_string_pretty(&definition).unwrap() + "\n";
    assert_eq!(legacy, old_writer.as_bytes());
    let legacy_value: Value = serde_json::from_slice(&legacy).unwrap();
    assert_eq!(legacy_value["format"], "pcbcraft-footprint/1");
    assert!(legacy_value.get("non_plated_holes").is_none());

    definition.non_plated_holes.push(CircularNpth { center: Point { x_nm: MAX_OFFSET_NM, y_nm: -MAX_OFFSET_NM }, diameter_nm: MAX_OFFSET_NM });
    assert_eq!(write_saved(&definition).unwrap_err(), "footprint format does not match its non-plated hole content");
    definition.format = FootprintDefinition::format_for(true).into();
    let bytes = write_saved(&definition).unwrap();
    assert_eq!(*read_saved(&bytes).unwrap(), definition);
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    let invalid_geometry = "invalid circular non-plated hole count, centre or finished diameter";
    for (invalid, expected) in [
        (json!([]), "expected 1–128 non-plated holes"),
        (Value::Null, "expected a sequence"),
        (json!(vec![value["non_plated_holes"][0].clone(); MAX_MECHANICAL_HOLES + 1]), "expected 1–128 non-plated holes"),
        (json!([{"center":{"x_nm":0,"y_nm":0},"diameter_nm":0}]), invalid_geometry),
        (json!([{"center":{"x_nm":0,"y_nm":0},"diameter_nm":MAX_OFFSET_NM + 1}]), invalid_geometry),
        (json!([{"center":{"x_nm":i64::MIN,"y_nm":0},"diameter_nm":1}]), invalid_geometry),
        (json!([{"center":{"x_nm":0,"y_nm":0},"diameter_nm":1,"number":"1"}]), "unknown field `number`"),
    ] {
        let mut changed = value.clone();
        changed["non_plated_holes"] = invalid;
        let error = read_saved(&serde_json::to_vec(&changed).unwrap()).unwrap_err();
        assert!(error.contains(expected), "unexpected error: {error}");
    }
    let mut downgraded = value.clone();
    downgraded["format"] = json!(LEGACY_FORMAT);
    let mut empty = legacy_value;
    empty["format"] = json!(NPTH_FORMAT);
    for invalid in [downgraded, empty] {
        assert_eq!(read_saved(&serde_json::to_vec(&invalid).unwrap()).unwrap_err(), "footprint format does not match its non-plated hole content");
    }
    definition.non_plated_holes.clear();
    definition.format = FootprintDefinition::format_for(false).into();
    assert_eq!(write_saved(&definition).unwrap(), legacy);
}

/// Author two physical holes next to two mapped electrical pads, then place the
/// package using the same native authoring and board transactions as clients.
#[test]
fn circular_npth_is_mechanical_through_transform_copy_history_and_reopen() {
    use crate::authoring::{Action, Item, Kind, Primitive};
    use crate::board::{clearance::ObstacleId, clipboard, geometry::Contour};
    let mut editor = crate::authoring::Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let legacy = editor.snapshot().encode().unwrap();
    let centers = [Point { x_nm: 3_000_000, y_nm: 5_000_000 }, Point { x_nm: -4_000_000, y_nm: 6_000_000 }];
    for center in centers {
        editor
            .execute(
                editor.revision(),
                Action::Insert { primitive: Primitive::NonPlatedHole { hole: CircularNpth { center, diameter_nm: 3_200_000 } } },
            )
            .unwrap();
    }
    let before = editor.snapshot();
    let draft = before.encode().unwrap();
    assert_eq!(Item::decode(&draft).unwrap(), *before);
    assert_eq!(serde_json::from_slice::<Value>(&legacy).unwrap()["format"], "pcbcraft-library-item/1");
    let mut wrong_envelope = serde_json::from_slice::<Value>(&draft).unwrap();
    assert_eq!(wrong_envelope["format"], "pcbcraft-library-item/3");
    wrong_envelope["format"] = json!("pcbcraft-library-item/1");
    assert_eq!(Item::decode(&serde_json::to_vec(&wrong_envelope).unwrap()).unwrap_err(), "Invalid or oversized native library item.");
    let current = editor.revision();
    let invalid = Action::Insert { primitive: Primitive::NonPlatedHole { hole: CircularNpth { center: centers[0], diameter_nm: 0 } } };
    for (revision, expected) in [
        (current - 1, "Library document changed; inspect it before editing."),
        (current, "invalid circular non-plated hole count, centre or finished diameter"),
    ] {
        assert_eq!(editor.execute(revision, invalid.clone()).unwrap_err(), expected);
        assert_eq!(editor.revision(), current);
        assert!(Arc::ptr_eq(&editor.snapshot(), &before));
    }
    editor.execute(current, Action::Undo).unwrap();
    assert_eq!(editor.snapshot().footprint().unwrap().non_plated_holes.len(), 1);
    editor.execute(editor.revision(), Action::Redo).unwrap();
    let definition = editor.snapshot().footprint().unwrap();
    assert_eq!(definition.non_plated_holes.len(), 2);
    assert_eq!(Item::edit_footprint(&definition).unwrap().footprint().unwrap().non_plated_holes, definition.non_plated_holes);

    let mut s = session();
    let manifest =
        json!({"format":crate::library::LIBRARY_FORMAT,"id":definition.library,"name":"Synthetic mounting fixture","license":definition.license})
            .to_string();
    let native = editor.snapshot().export_native().unwrap().to_string();
    s.execute("library.load_source", &json!({"source":"mounting-fixture","documents":{"library.json":manifest,"footprints/mounting.json":native}}))
        .unwrap();
    symbol(&mut s, "resistor");
    let component = s.project().unwrap().components().next().unwrap().id();
    s.apply(crate::ASSIGN_FOOTPRINT, &json!({"component":component,"library":definition.library,"definition":definition.id,"definition_revision":definition.revision,"mapping":{"1":"2","2":"1"},"expected_revision":s.revision()})).unwrap();
    s.apply(crate::INITIALIZE_UPDATE_BASELINE, &json!({"expected_revision":s.revision()})).unwrap();
    s.apply(crate::PLACE_ASSIGNED_FOOTPRINT, &json!({"component":component,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let target = s.project().unwrap().board_target().unwrap();
    let view = |p: &crate::Project| p.board_view(target).unwrap().footprints().next().unwrap().non_plated_holes;
    assert_eq!(view(s.project().unwrap()), definition.non_plated_holes);
    let original = s.snapshot().unwrap();
    let id = original.board_view(target).unwrap().footprints().next().unwrap().id;
    let project = s.inspect();
    let inspected = &project["project"]["board"]["footprints"][0];
    assert_eq!(inspected["pins"].as_array().unwrap().len(), 2);
    for hole in inspected["non_plated_holes"].as_array().unwrap() {
        assert_eq!(hole["span"], "through");
        assert_eq!(hole["plating"], "non_plated");
        assert_eq!(hole.get("net"), Some(&Value::Null));
        assert!(hole.get("number").is_none());
    }
    let board = original.board_view(target).unwrap();
    assert!(board.ratsnest().unwrap().lines.is_empty());
    assert_eq!(board.footprints().next().unwrap().pads.len(), 2);
    assert!(board.fabrication_outputs().unwrap_err().contains("does not yet support non-plated holes"));
    let query = s.clearance_query(target, 0).unwrap();
    let obstacles: Vec<_> = query.objects().iter().enumerate().filter(|(_, item)| item.mechanical).collect();
    assert_eq!(obstacles.len(), 2);
    for (index, item) in obstacles {
        assert!(item.intended_nets.is_empty());
        assert_eq!(item.layers.len(), 2);
        let at = item.at;
        // 1.6 mm finished radius and 0.1 mm probe radius give exact tangency.
        let at = Point { x_nm: at.x_nm + 1_700_000, y_nm: at.y_nm };
        let probe = Contour::capsule(at, at, 100_000.);
        for &layer in &item.layers {
            let results = query.probe(&probe, &[layer], None).unwrap();
            assert!(results.decisions.iter().any(|d| d.second == index && d.touches && d.clearance_violation && !d.conflicting_nets));
            let overlay = query.overlay().probe(&probe, &[layer], None).unwrap();
            assert!(
                overlay
                    .decisions
                    .iter()
                    .any(|d| { d.obstacle == ObstacleId::Source(index) && d.touches && d.clearance_violation && !d.conflicting_nets })
            );
            let err = query.probe(&probe, &[layer], Some(query.objects().len())).unwrap_err();
            assert_eq!(err.code(), crate::error_codes::ErrorCode::NotFound);
        }
        let request = crate::board::route::Request {
            head: item.at,
            target: Point { x_nm: item.at.x_nm + 10_000_000, y_nm: item.at.y_nm },
            layer: item.layers[0],
            origin: None,
            width_nm: 200_000,
            posture: crate::board::route::Posture::default(),
            flipped: false,
            previous: Vec::new(),
        };
        let plan = s.plan_route(&query, &request, 128, &|| false).unwrap();
        assert_eq!(plan.outcome, crate::board::route::Outcome::Blocked { last_legal: item.at });
        assert_eq!(plan.blocker.as_deref(), Some(item.name.as_str()));
    }

    // A thin route may end here, but a wide via at its fixed head reaches the opening.
    let layer = query.objects().iter().find(|item| item.mechanical).unwrap().layers[0];
    let at = Point { x_nm: centers[0].x_nm + 2_100_000, y_nm: centers[0].y_nm };
    let context = s.route_planning(target).unwrap();
    assert_eq!(context.try_through_via_clears_mechanical(&s, at, 1_200_000).unwrap(), None);
    let request = crate::board::route::Request {
        head: at,
        target: at,
        layer,
        origin: None,
        width_nm: 200_000,
        posture: crate::board::route::Posture::default(),
        flipped: false,
        previous: Vec::new(),
    };
    let mut job = s.prepare_route_plan(&context, request, 128).unwrap();
    assert!((0..256).any(|_| job.step(32)), "synthetic fixed-head planning finishes");
    let plan = job.take_result().unwrap().unwrap();
    assert_eq!(plan.outcome, crate::board::route::Outcome::Reached);
    assert_eq!(plan.points, vec![at]);
    assert_eq!(context.try_through_via_clears_mechanical(&s, at, 300_000).unwrap(), Some(true));
    assert_eq!(context.try_through_via_clears_mechanical(&s, at, 1_200_000).unwrap(), Some(false));

    s.apply(SET_FOOTPRINT_TRANSFORM, &json!({"footprint":id,"x_nm":0,"y_nm":0,"rotation":90,"side":"back","expected_revision":s.revision()}))
        .unwrap();
    let transformed = view(s.project().unwrap());
    assert_eq!(transformed.iter().map(|h| h.center).collect::<Vec<_>>(), centers.map(|c| Point { x_nm: c.y_nm, y_nm: c.x_nm }));
    let before_copy = s.snapshot().unwrap();
    let result = s.execute(clipboard::COPY, &json!({"items":{"footprints":[id]},"expected_revision":s.revision()})).unwrap();
    let text = result["clipboard"].as_str().unwrap();
    let mut envelope: Value = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["format"], "pcbcraft-clipboard/7");
    let nested = envelope["circuits"][0]["text"].as_str().unwrap();
    let mut nested_envelope: Value = serde_json::from_str(nested).unwrap();
    assert_eq!(nested_envelope["format"], "pcbcraft-clipboard/6");
    nested_envelope["format"] = json!("pcbcraft-clipboard/1");
    assert!(matches!(crate::clipboard::Fragment::from_text(&nested_envelope.to_string()),
        Err(crate::CommandError::InvalidParams(message)) if message.contains("format does not match its pinned non-plated holes")));
    envelope["format"] = json!("pcbcraft-clipboard/1");
    assert!(matches!(clipboard::BoardFragment::from_text(&envelope.to_string()),
        Err(crate::CommandError::InvalidParams(message)) if message.contains("format does not match its pinned non-plated holes")));
    let fragment = clipboard::BoardFragment::from_text(text).unwrap();
    assert_eq!(fragment.to_text().unwrap(), text);
    let sheet = s.project().unwrap().sheet_target().id();
    let sources: std::collections::BTreeMap<_, _> = fragment
        .circuits()
        .iter()
        .map(|c| (c.source_sheet, clipboard::SchematicDestination { sheet, at: Point { x_nm: 0, y_nm: 100_000_000 } }))
        .collect();
    s.apply(clipboard::PASTE, &json!({"text":text,"x_nm":30_000_000,"y_nm":10_000_000,"schematic_targets":sources,"expected_revision":s.revision()}))
        .unwrap();
    let after_copy = s.snapshot().unwrap();
    let pasted = after_copy.board_view(target).unwrap().footprints().last().unwrap();
    assert!(pasted.bottom);
    assert_eq!(pasted.rotation, 90);
    assert_eq!(
        pasted.non_plated_holes,
        transformed
            .iter()
            .map(|hole| CircularNpth { center: Point { x_nm: hole.center.x_nm + 30_000_000, y_nm: hole.center.y_nm + 10_000_000 }, ..*hole })
            .collect::<Vec<_>>()
    );
    let bytes = encode_documents(after_copy.clone()).unwrap();
    for reopened in [decode_documents(&bytes).unwrap(), decode_packed(&encode_packed(after_copy.clone()).unwrap()).unwrap().project] {
        assert_eq!(encode_documents(reopened.clone()).unwrap(), bytes);
        assert_eq!(reopened.board_view(target).unwrap().footprints().last().unwrap().non_plated_holes, pasted.non_plated_holes);
    }
    s.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&before_copy, &s.snapshot().unwrap());
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);

    // Let the shared direct edit create a DRC case without relying on the interactive walker.
    s.apply(crate::ADD_VIA, &json!({"at":transformed[0].center,"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":s.revision()}))
        .unwrap();
    use crate::board::drc;
    let params = json!({"board":target.board});
    let sync = s.check_design(&params).unwrap();
    let mut job = s.design_check_job(&params).unwrap();
    for _ in 0..128 {
        if job.step(16) {
            break;
        }
    }
    let report = job.take_report().unwrap();
    assert!(report.violations.iter().any(|v| v.kind == drc::Kind::Clearance && v.message == "Non-plated hole touches copper" && v.nets.is_empty()));
    assert_eq!(json!(report), sync);
}

/// Library defaults and board adjustments compose before clipping, while authoring preserves pads.
#[test]
fn library_and_board_reference_offsets_are_additive() {
    use crate::authoring::{Action, FootprintReferenceSettings, Item};
    let original = basic_components("resistor-0402").unwrap();
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
    for id in BASIC_COMPONENT_IDS {
        basic_components(id).unwrap().validate().unwrap();
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
    assert!(bytes.contains_key("library/footprints/basic-components/resistor-0402.r4.json"));
    assert!(bytes.contains_key("library/footprints/basic-components/sot23.r4.json"));
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
    let original = "library/footprints/basic-components/resistor-0603.r4.json";
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
    assert!(basic_components("custom-resistor").is_none());
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
    let path = "library/footprints/basic-components/sot23.r4.json";
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
    let path = "library/footprints/basic-components/header-1x02-254.r3.json";
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
        for (index, (footprint, _)) in footprints.iter().zip(&sheets).enumerate() {
            assert_eq!(footprint.pads.iter().find(|pad| pad.number == "2").unwrap().net, Some(format!("Net-{:04}", index + 1)));
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

/// Expanded defaults retain exact native maps and offline geometry, including header pitch and polarity.
#[test]
fn expanded_defaults_place_and_reopen_without_installed_sources() {
    let mut s = session();
    let mut x = 0;
    for (symbol_id, footprint_id, map) in [
        ("resistor", "resistor-0805", json!({"1":"1","2":"2"})),
        ("resistor", "resistor-1206", json!({"1":"1","2":"2"})),
        ("diode", "sod123", json!({"1":"1","2":"2"})),
        ("npn", "sot323", json!({"1":"1","2":"2","3":"3"})),
        ("header-3", "header-1x03-254", json!({"1":"1","2":"2","3":"3"})),
        ("header-4", "header-1x04-254", json!({"1":"1","2":"2","3":"3","4":"4"})),
        ("header-6", "header-1x06-254", json!({"1":"1","2":"2","3":"3","4":"4","5":"5","6":"6"})),
        ("header-8", "header-1x08-254", json!({"1":"1","2":"2","3":"3","4":"4","5":"5","6":"6","7":"7","8":"8"})),
        ("header-2x3", "header-2x03-254", json!({"1":"1","2":"2","3":"3","4":"4","5":"5","6":"6"})),
    ] {
        let definition = s.libraries().latest_symbol("basic-components", symbol_id).unwrap();
        s.apply(PLACE_SYMBOL, &json!({"library":"basic-components","definition":symbol_id,"definition_revision":definition.revision(),"definition_digest":crate::library::sources::symbol_digest(&definition).unwrap(),"x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
        let id = s.project().unwrap().symbols().last().unwrap().id();
        assign(&mut s, &id, footprint_id, map);
        x += 20_000_000;
    }
    let pinned = s.snapshot().unwrap();
    let documents = encode_documents(pinned.clone()).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened).unwrap(), documents);
    s.set_builtin_library_enabled("basic-components", false).unwrap();
    assert!(Arc::ptr_eq(&s.snapshot().unwrap(), &pinned));
    let dual = basic_components("header-2x03-254").unwrap();
    assert_eq!(dual.pads[0].center, Point { x_nm: -1_270_000, y_nm: -2_540_000 });
    assert_eq!(dual.pads[1].center, Point { x_nm: 1_270_000, y_nm: -2_540_000 });
    assert_eq!(dual.pads[0].shape, PadShape::Rectangle);
    assert_eq!(basic_components("sod123").unwrap().pads[0].number, "1");
}
