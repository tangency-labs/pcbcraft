use super::*;
use crate::{
    ASSIGN_FOOTPRINT, CONNECT_PINS, INITIALIZE_BOARD_SETUP, INITIALIZE_UPDATE_BASELINE, PLACE_ASSIGNED_FOOTPRINT, PLACE_SYMBOL,
    persistence::{decode_documents, encode_documents},
    schematic::GRID_NM,
};

/// Two command-created modern packages and an accepted internal connection; both pad maps are
/// intentionally permuted so copies cannot infer terminal correspondence from display numbers.
fn fixture() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Board clipboard","create_board":true})).unwrap();
    for x in [0, 20 * GRID_NM] {
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    let symbols: Vec<_> = s.project().unwrap().symbols().iter().map(|p| p.id()).collect();
    s.apply(
        CONNECT_PINS,
        &json!({"from":{"symbol":symbols[0],"terminal":"2"},"to":{"symbol":symbols[1],"terminal":"1"},"expected_revision":s.revision()}),
    )
    .unwrap();
    let components: Vec<_> = s.project().unwrap().components().map(|c| c.id()).collect();
    for id in &components {
        s.apply(ASSIGN_FOOTPRINT,&json!({"component":id,"library":"starter","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"2","2":"1"},"expected_revision":s.revision()})).unwrap();
    }
    s.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    s.apply(INITIALIZE_UPDATE_BASELINE, &json!({"expected_revision":s.revision()})).unwrap();
    for (index, id) in components.iter().enumerate() {
        s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":id,"x_nm":index as i64*10_000_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    s
}

/// An explicit default view for this one-board fixture.
fn view(s: &Session) -> super::super::view::BoardView<'_> {
    let p = s.project().unwrap();
    p.board_view(p.board_target().unwrap()).unwrap()
}
/// Select every currently authored physical object.
fn all(s: &Session) -> BoardSelection {
    let board = view(s);
    BoardSelection {
        footprints: board.footprints().map(|f| f.id).collect(),
        drawings: board.drawings().iter().map(|d| d.id).collect(),
        tracks: board.tracks().iter().map(|t| t.id).collect(),
        vias: board.vias().iter().map(|v| v.id).collect(),
    }
}
/// Submit a duplicate at a distinct physical and schematic anchor with explicit source-sheet mapping.
fn paste_params(s: &Session, text: &str) -> Value {
    let fragment = BoardFragment::from_text(text).unwrap();
    let sheet = s.project().unwrap().sheet_target().id();
    let targets: BTreeMap<_, _> =
        fragment.circuits().iter().map(|c| (c.source_sheet, SchematicDestination { sheet, at: Point { x_nm: 0, y_nm: 30 * GRID_NM } })).collect();
    json!({"text":text,"x_nm":0,"y_nm":10_000_000,"schematic_targets":targets,"expected_revision":s.revision()})
}

/// Board duplication creates linked schematic/physical identities, preserves the internal net
/// and exact pin permutation, accepts only this addition and remains one undo/redo/save operation.
#[test]
fn duplicated_footprints_and_circuit_are_one_atomic_edit() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let result = s.execute(COPY, &json!({"items":all(&s),"expected_revision":s.revision()})).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let text = result["clipboard"].as_str().unwrap();
    let fragment = BoardFragment::from_text(text).unwrap();
    assert_eq!(fragment.counts(), (2, 0, 0, 0));
    assert_eq!(fragment.circuits()[0].fragment.counts(), (2, 1, 0));
    let params = paste_params(&s, text);
    s.apply(PASTE, &params).unwrap();
    let p = s.project().unwrap();
    assert_eq!(p.symbols().len(), 4);
    assert_eq!(p.board.footprints().len(), 4);
    assert_eq!(p.wires().len(), 2);
    assert_eq!(p.board.baseline.as_ref().unwrap().components.len(), 4);
    for footprint in &p.board.footprints()[2..] {
        assert!(footprint.component > before.highest_id());
        assert_eq!(footprint.mapping.as_ref().unwrap()["1"], "2");
        assert_eq!(footprint.y_nm, 10_000_000);
    }
    assert!(
        p.board
            .baseline
            .as_ref()
            .unwrap()
            .nets
            .iter()
            .any(|n| n.terminals.len() == 2 && n.terminals.iter().all(|t| t.component > before.highest_id()))
    );
    let after = s.snapshot().unwrap();
    let documents = encode_documents(after.clone()).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    s.apply("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&after, &s.snapshot().unwrap());
}

/// Clipboard transport retains a bottom-side pose through group rotation, native format 14
/// round-tripping and atomic undo, alongside an ordinary top-side footprint.
#[test]
fn bottom_side_survives_copy_rotate_paste_and_undo() {
    let mut s = fixture();
    let footprint = view(&s).footprints().next().unwrap();
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":footprint.id,"x_nm":footprint.position.x_nm,"y_nm":footprint.position.y_nm,"rotation":90,"side":"back","expected_revision":s.revision()}),
    )
    .unwrap();
    let before = s.snapshot().unwrap();
    let copied = s.execute(COPY, &json!({"items":all(&s),"expected_revision":s.revision()})).unwrap();
    let mut params = paste_params(&s, copied["clipboard"].as_str().unwrap());
    params["rotation"] = json!(90);
    s.apply(PASTE, &params).unwrap();
    let after = s.snapshot().unwrap();
    assert_eq!(after.board.footprints().len(), 4);
    for (original, pasted) in before.board.footprints().iter().zip(&after.board.footprints()[2..]) {
        assert_eq!(pasted.bottom, original.bottom);
        assert_eq!(pasted.rotation, (original.rotation + 90) % 360);
    }
    let documents = encode_documents(after).unwrap();
    assert!(String::from_utf8(documents["board.json"].clone()).unwrap().contains("pcbcraft-board/14"));
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
}

/// A physical cut keeps the original components, circuit and accepted package state, and same-
/// project paste reuses those identities while allocating fresh physical placement IDs.
#[test]
fn cut_paste_relocates_existing_components_without_duplicating_the_schematic() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let items = all(&s);
    let copied = s.execute(CUT, &json!({"items":items,"expected_revision":s.revision()})).unwrap();
    assert!(s.project().unwrap().board.footprints().is_empty());
    assert_eq!(s.project().unwrap().symbols().len(), 2);
    let after_cut = s.snapshot().unwrap();
    s.apply(
        PASTE,
        &json!({"text":copied["clipboard"],"reuse_components":true,"x_nm":5_000_000,"y_nm":8_000_000,"rotation":90,"expected_revision":s.revision()}),
    )
    .unwrap();
    let p = s.project().unwrap();
    assert_eq!(p.components.len(), 2);
    assert_eq!(p.wires().len(), 1);
    assert_eq!(p.board.footprints().len(), 2);
    for (old, new) in before.board.footprints().iter().zip(p.board.footprints()) {
        assert_eq!(old.component, new.component);
        assert_ne!(old.id, new.id);
        assert_eq!(new.rotation, 90);
    }
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&after_cut, &s.snapshot().unwrap());
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
}

/// Cross-project duplication resolves pinned assets offline and never reuses source components.
#[test]
fn cross_project_copy_needs_explicit_schematic_ownership() {
    let source = fixture();
    let text = BoardFragment::capture(source.project().unwrap(), &all(&source), false).unwrap().to_text().unwrap();
    let mut target = Session::default();
    target.apply("project.new", &json!({"create_board":true})).unwrap();
    let before = target.snapshot().unwrap();
    assert!(target.apply(PASTE, &json!({"text":text,"x_nm":0,"y_nm":0,"expected_revision":target.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &target.snapshot().unwrap()));
    let params = paste_params(&target, &text);
    target.apply(PASTE, &params).unwrap();
    assert_eq!(target.project().unwrap().components.len(), 2);
    assert_eq!(target.project().unwrap().board.footprints().len(), 2);
    assert_eq!(target.project().unwrap().board.definitions.len(), 1);
    let before = target.snapshot().unwrap();
    assert!(target.apply(PASTE, &json!({"text":text,"reuse_components":true,"x_nm":0,"y_nm":0,"expected_revision":target.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &target.snapshot().unwrap()));
}

/// Pasting a duplicate cannot accept unrelated pending schematic changes on the destination board.
#[test]
fn unrelated_pending_board_updates_cannot_be_accepted_by_paste() {
    let mut s = fixture();
    let text = BoardFragment::capture(s.project().unwrap(), &all(&s), false).unwrap().to_text().unwrap();
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":40*GRID_NM,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let before = s.snapshot().unwrap();
    let error = s.apply(PASTE, &paste_params(&s, &text)).unwrap_err().to_string();
    assert!(error.contains("pending PCB update"), "{error}");
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Unknown targets/identities, stale revisions, reuse of placed objects and reference conflicts
/// preserve the complete snapshot and history, including accepted capture identity.
#[test]
fn refused_board_pastes_publish_nothing() {
    let mut s = fixture();
    let text = BoardFragment::capture(s.project().unwrap(), &all(&s), true).unwrap().to_text().unwrap();
    let before = s.snapshot().unwrap();
    let base = paste_params(&s, &text);
    for extra in [
        json!({"references":"preserve"}),
        json!({"expected_revision":0}),
        json!({"board":Uuid::nil()}),
        json!({"x_nm":999_999_999}),
        json!({"text":"ordinary clipboard text"}),
    ] {
        let mut params = base.clone();
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(s.apply(PASTE, &params).is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    assert!(s.apply(PASTE, &json!({"text":text,"reuse_components":true,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert!(s.apply(CUT, &json!({"items":{"footprints":["footprint-999999"]},"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Add a routed two-via connection and one original outline through shared commands.
fn add_geometry(s: &mut Session) {
    use super::super::setup::Side;
    let board = view(s);
    let line = board.ratsnest().unwrap().lines[0].clone();
    let layer = |side| board.layers().into_iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(side)).unwrap().id;
    let front = layer(Side::Front);
    let back = layer(Side::Back);
    let points = [
        line.from.position,
        Point { x_nm: line.from.position.x_nm, y_nm: 2_000_000 },
        Point { x_nm: line.to.position.x_nm, y_nm: 2_000_000 },
        line.to.position,
    ];
    let outline = board.layers().into_iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id;
    s.apply(crate::ADD_TRACKS,&json!({"layer":front,"points":points,"width_nm":250_000,"layer_changes":[{"at":1,"layer":back},{"at":2,"layer":front}],"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":s.revision()})).unwrap();
    s.apply(crate::ADD_DRAWING,&json!({"layer":outline,"points":[{"x_nm":-2_000_000,"y_nm":-3_000_000},{"x_nm":12_000_000,"y_nm":-3_000_000}],"closed":false,"expected_revision":s.revision()})).unwrap();
}

/// Routed circuits copy as one group, including vias and outline geometry. The copied copper
/// joins only its own remapped logical pads; accepted membership and native bytes round-trip.
#[test]
fn mixed_circuit_copper_vias_and_outline_paste_together() {
    let mut s = fixture();
    add_geometry(&mut s);
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), 0);
    let before = s.snapshot().unwrap();
    let text = BoardFragment::capture(&before, &all(&s), false).unwrap().to_text().unwrap();
    let fragment = BoardFragment::from_text(&text).unwrap();
    assert_eq!(fragment.counts(), (2, 1, 3, 2));
    s.apply(PASTE, &paste_params(&s, &text)).unwrap();
    assert_eq!((view(&s).tracks().len(), view(&s).vias().len(), view(&s).drawings().len()), (6, 4, 2));
    assert_eq!(view(&s).ratsnest().unwrap().lines.len(), 0);
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(std::str::from_utf8(&documents["board.json"]).unwrap().contains("pcbcraft-board/13"));
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
}

/// Copper/graphics-only clips create no schematic data or numeric allocation churn. A via-only
/// cut and paste works without manufacturing a placeholder component or a layer address.
#[test]
fn geometry_only_copy_cut_and_paste_do_not_create_components() {
    let mut s = fixture();
    add_geometry(&mut s);
    let via = view(&s).vias()[0].id;
    let before = s.snapshot().unwrap();
    let selected = BoardSelection { vias: [via].into(), ..Default::default() };
    let text = s.execute(CUT, &json!({"items":selected,"expected_revision":s.revision()})).unwrap()["clipboard"].as_str().unwrap().to_owned();
    assert_eq!(view(&s).vias().len(), 1);
    s.apply(PASTE, &json!({"text":text,"x_nm":20_000_000,"y_nm":20_000_000,"expected_revision":s.revision()})).unwrap();
    assert_eq!(view(&s).vias().len(), 2);
    assert_eq!(s.project().unwrap().components.len(), 2);
    assert_eq!(s.project().unwrap().numeric_id_high_water, before.numeric_id_high_water);
    let source_docs = encode_documents(before).unwrap();
    let after_docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(source_docs["project.pc"], after_docs["project.pc"]);
}

/// Foreign layers map by typed role/side, never by display name. Ambiguous internal copper
/// requires an explicit compatible destination and unknown/extra layer mappings are refused.
#[test]
fn cross_board_layers_are_explicit_when_roles_are_ambiguous() {
    use super::super::setup::Side;
    let mut source = fixture();
    add_geometry(&mut source);
    let items = BoardSelection {
        tracks: view(&source).tracks().iter().map(|t| t.id).collect(),
        drawings: view(&source).drawings().iter().map(|d| d.id).collect(),
        ..Default::default()
    };
    let mut fragment = BoardFragment::capture(source.project().unwrap(), &items, false).unwrap();
    let mut target = Session::default();
    target.apply("project.new", &json!({"create_board":true})).unwrap();
    target.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":target.revision()})).unwrap();
    let mapping = fragment.layer_mapping(target.project().unwrap(), &BTreeMap::new()).unwrap();
    for layer in fragment.layers() {
        let destination = view(&target).layers().into_iter().find(|l| l.id == mapping[&layer.id]).unwrap();
        assert_eq!(destination.purpose, layer.purpose);
        assert_eq!(destination.side, layer.side);
    }
    let source_front = fragment.layers.iter_mut().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Front)).unwrap();
    source_front.side = None;
    let source = source_front.id;
    assert!(fragment.layer_mapping(target.project().unwrap(), &BTreeMap::new()).is_err());
    let inner = view(&target).layers().into_iter().find(|l| l.purpose == LayerPurpose::Copper && l.side.is_none()).unwrap().id;
    assert_eq!(fragment.layer_mapping(target.project().unwrap(), &BTreeMap::from([(source, inner)])).unwrap()[&source], inner);
    assert!(fragment.layer_mapping(target.project().unwrap(), &BTreeMap::from([(Uuid::nil(), inner)])).is_err());
}

/// Copied copper cannot bridge two different accepted nets at its destination, and stale or
/// malformed layer geometry cannot leave an outline, via or history entry partially inserted.
#[test]
fn new_copper_shorts_reject_the_whole_batch() {
    use super::super::setup::Side;
    let mut target = fixture();
    let ids: Vec<_> = target.project().unwrap().symbols().iter().map(|s| s.id()).collect();
    target
        .apply(
            CONNECT_PINS,
            &json!({"from":{"symbol":ids[0],"terminal":"1"},"to":{"symbol":ids[1],"terminal":"2"},"expected_revision":target.revision()}),
        )
        .unwrap();
    let review = target.execute(crate::PREVIEW_UPDATE, &json!({"expected_revision":target.revision()})).unwrap();
    target
        .apply(crate::APPLY_UPDATE, &json!({"plan_token":review["board_update_preview"]["plan_token"],"expected_revision":target.revision()}))
        .unwrap();
    let pads = view(&target).footprints().next().unwrap().pads;
    let a = pads[0].center;
    let b = pads[1].center;
    assert_ne!(pads[0].net, pads[1].net);
    let mut source = Session::default();
    source.apply("project.new", &json!({"create_board":true})).unwrap();
    source.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":source.revision()})).unwrap();
    let layer = view(&source).layers().into_iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(Side::Front)).unwrap().id;
    source.apply(crate::ADD_TRACKS,&json!({"layer":layer,"points":[{"x_nm":0,"y_nm":0},{"x_nm":b.x_nm-a.x_nm,"y_nm":b.y_nm-a.y_nm}],"width_nm":250_000,"expected_revision":source.revision()})).unwrap();
    let text = BoardFragment::capture(source.project().unwrap(), &all(&source), false).unwrap().to_text().unwrap();
    let before = target.snapshot().unwrap();
    let error = target.apply(PASTE, &json!({"text":text,"x_nm":a.x_nm,"y_nm":a.y_nm,"expected_revision":target.revision()})).unwrap_err().to_string();
    assert!(error.contains("connect nets"), "{error}");
    assert!(Arc::ptr_eq(&before, &target.snapshot().unwrap()));
}

/// A native excluded power marker is logical annotation, not an extra physical package. Its
/// name and connection travel with the copied circuit and can be isolated by the suffix option.
#[test]
fn copied_circuits_retain_attached_native_power_annotations() {
    let mut s = fixture();
    let pin = s.project().unwrap().symbols()[0].pins()[1];
    let definition = s.libraries.latest_symbol("power", "gnd").unwrap();
    s.apply(PLACE_SYMBOL,&json!({"library":"power","definition":"gnd","definition_revision":definition.revision(),"definition_digest":crate::library::sources::symbol_digest(&definition).unwrap(),"x_nm":pin.x_nm,"y_nm":pin.y_nm,"value":"GND","expected_revision":s.revision()})).unwrap();
    let text = BoardFragment::capture(s.project().unwrap(), &all(&s), false).unwrap().to_text().unwrap();
    let fragment = BoardFragment::from_text(&text).unwrap();
    assert_eq!(fragment.circuits()[0].fragment.counts().0, 3);
    assert_eq!(fragment.counts().0, 2);
    let mut target = Session::default();
    target.apply("project.new", &json!({"create_board":true})).unwrap();
    let mut params = paste_params(&target, &text);
    params["net_name_suffix"] = json!("_B");
    target.apply(PASTE, &params).unwrap();
    assert_eq!(target.project().unwrap().symbols().len(), 3);
    assert_eq!(target.project().unwrap().board.footprints().len(), 2);
    assert!(target.project().unwrap().components().any(|c| c.definition.power_terminal().is_some() && c.value() == "GND_B"));
}

/// Footprints from several source sheets retain separate circuit fragments. The destination map
/// can deliberately swap their homes, and a suffix preserves one isolated cross-sheet global net.
#[test]
fn multisheet_board_copy_keeps_explicit_schematic_destinations() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let first = s.project().unwrap().sheet_target();
    s.apply(crate::sheets::CREATE_SHEET, &json!({"name":"Second","expected_revision":s.revision()})).unwrap();
    let second = s.project().unwrap().sheets().nth(1).unwrap().target;
    for (sheet, terminal) in [(first, 1), (second, 0)] {
        s.apply(PLACE_SYMBOL, &json!({"sheet":sheet.id(),"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
        let project = s.project().unwrap().for_sheet(sheet).unwrap();
        let symbol = &project.symbols()[0];
        let pin = symbol.pins()[terminal];
        s.apply(
            crate::net_labels::ADD,
            &json!({"sheet":sheet.id(),"name":"LINK","style":"global","x_nm":pin.x_nm,"y_nm":pin.y_nm,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    let components: Vec<_> = s.project().unwrap().components().map(|c| c.id()).collect();
    for component in &components {
        s.apply(ASSIGN_FOOTPRINT,&json!({"component":component,"library":"starter","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"1","2":"2"},"expected_revision":s.revision()})).unwrap();
    }
    s.apply(INITIALIZE_UPDATE_BASELINE, &json!({"expected_revision":s.revision()})).unwrap();
    for (index, component) in components.iter().enumerate() {
        s.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":component,"x_nm":index as i64*10_000_000,"y_nm":0,"expected_revision":s.revision()}))
            .unwrap();
    }
    let fragment = BoardFragment::capture(s.project().unwrap(), &all(&s), false).unwrap();
    assert_eq!(fragment.circuits().len(), 2);
    let text = fragment.to_text().unwrap();
    let targets = BTreeMap::from([
        (first.id(), SchematicDestination { sheet: second.id(), at: Point { x_nm: 30 * GRID_NM, y_nm: 0 } }),
        (second.id(), SchematicDestination { sheet: first.id(), at: Point { x_nm: 40 * GRID_NM, y_nm: 0 } }),
    ]);
    let before = s.snapshot().unwrap();
    s.apply(
        PASTE,
        &json!({"text":text,"x_nm":0,"y_nm":10_000_000,"schematic_targets":targets,"net_name_suffix":"_B","expected_revision":s.revision()}),
    )
    .unwrap();
    let p = s.project().unwrap();
    assert_eq!(p.for_sheet(first).unwrap().symbols()[1].position.x_nm, 40 * GRID_NM);
    assert_eq!(p.for_sheet(second).unwrap().symbols()[1].position.x_nm, 30 * GRID_NM);
    assert!(p.board.baseline.as_ref().unwrap().nets.iter().any(|n| n.names.contains(&"LINK_B".into()) && n.terminals.len() == 2));
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
}

/// Structural clipboard corruption is rejected before destination mutation, including missing
/// circuit ownership, duplicate physical identities, invalid via sizes and unsupported versions.
#[test]
fn malformed_board_payloads_are_rejected_before_insertion() {
    let mut s = fixture();
    add_geometry(&mut s);
    let text = BoardFragment::capture(s.project().unwrap(), &all(&s), false).unwrap().to_text().unwrap();
    let original: Value = serde_json::from_str(&text).unwrap();
    let before = s.snapshot().unwrap();
    for mode in 0..5 {
        let mut value = original.clone();
        match mode {
            0 => value["format"] = json!("pcbcraft-clipboard/99"),
            1 => value["circuits"] = json!([]),
            2 => value["footprints"][1]["id"] = value["footprints"][0]["id"].clone(),
            3 => value["vias"][0]["drill_nm"] = value["vias"][0]["diameter_nm"].clone(),
            _ => value["layers"][0]["id"] = json!(Uuid::nil()),
        }
        let mut params = paste_params(&s, &text);
        params["text"] = json!(value.to_string());
        assert!(s.apply(PASTE, &params).is_err());
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    assert!(BoardSelection::parse(&json!({"tracks":[Uuid::nil(),Uuid::nil()]})).is_none());
}

/// Mixed deletion retains the established ability to remove pending physical remnants, even
/// when a logical component no longer exists and therefore cannot be exported as a circuit copy.
#[test]
fn physical_delete_does_not_require_a_live_schematic_counterpart() {
    let mut s = fixture();
    let symbol = s.project().unwrap().symbols()[0].id();
    let footprint = view(&s).footprints().next().unwrap().id;
    s.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbol,"expected_revision":s.revision()})).unwrap();
    let before = s.snapshot().unwrap();
    assert_eq!(view(&s).footprints().count(), 2);
    assert!(s.execute(CUT, &json!({"items":{"footprints":[footprint]},"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    s.apply(DELETE, &json!({"items":{"footprints":[footprint]},"expected_revision":s.revision()})).unwrap();
    assert_eq!(view(&s).footprints().count(), 1);
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    s.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
}

/// Replacing a cut component cannot make its retired numeric spelling refer to a new component.
#[test]
fn component_reuse_refuses_removed_source_identities() {
    let mut s = fixture();
    let copied = s.execute(CUT, &json!({"items":all(&s),"expected_revision":s.revision()})).unwrap();
    let symbol = s.project().unwrap().symbols()[0].id();
    s.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbol,"expected_revision":s.revision()})).unwrap();
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let before = s.snapshot().unwrap();
    assert!(s.apply(PASTE, &json!({"text":copied["clipboard"],"reuse_components":true,"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// A physical clip rejects logical annotations outside its copied component scope.
#[test]
fn unrelated_power_annotations_cannot_travel_in_circuit_context() {
    let mut s = fixture();
    let d = s.libraries.latest_symbol("power", "gnd").unwrap();
    s.apply(PLACE_SYMBOL,&json!({"library":"power","definition":"gnd","definition_revision":d.revision(),"definition_digest":crate::library::sources::symbol_digest(&d).unwrap(),"x_nm":50*GRID_NM,"y_nm":50*GRID_NM,"expected_revision":s.revision()})).unwrap();
    let text = BoardFragment::capture(s.project().unwrap(), &all(&s), false).unwrap().to_text().unwrap();
    let mut envelope: Envelope = serde_json::from_str(&text).unwrap();
    let p = s.project().unwrap();
    let selection =
        Selection { symbols: p.symbols().iter().map(|s| s.id()).collect(), wires: p.wires().iter().map(|w| w.id()).collect(), ..Default::default() };
    envelope.circuits[0].text = Fragment::capture(p, &selection).unwrap().to_text().unwrap();
    assert!(BoardFragment::from_text(&serde_json::to_string(&envelope).unwrap()).is_err());
}

/// Moving a cached ghost changes only its copied physical geometry, retains its preview IDs and
/// accepted-input capture, and never consumes session identities or modifies the source snapshot.
#[test]
fn cached_previews_move_without_reallocating_or_publishing() {
    let mut s = fixture();
    add_geometry(&mut s);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let fragment = BoardFragment::capture(&before, &all(&s), false).unwrap();
    let targets = fragment
        .circuits()
        .iter()
        .map(|c| (c.source_sheet, SchematicDestination { sheet: before.sheet_target().id(), at: Point { x_nm: 0, y_nm: 30 * GRID_NM } }))
        .collect();
    let options = BoardPasteOptions { schematic_targets: targets, ..Default::default() };
    let preview = fragment.preview(&before, s.next_object_id().unwrap(), &options).unwrap();
    let a = preview.at(Point { x_nm: 0, y_nm: 10_000_000 }, 0).unwrap();
    let b = preview.at(Point { x_nm: 20_000_000, y_nm: 20_000_000 }, 1).unwrap();
    assert_eq!(a.board.baseline.as_ref().unwrap().source, b.board.baseline.as_ref().unwrap().source);
    for id in &preview.selection().footprints {
        let id = crate::schematic::wiring::parse_id(id, "footprint-").unwrap();
        let fa = a.board.footprints().iter().find(|f| f.id == id).unwrap();
        let fb = b.board.footprints().iter().find(|f| f.id == id).unwrap();
        assert_eq!(fb.rotation, (fa.rotation + 90) % 360);
    }
    assert_eq!(a.components, b.components);
    assert_eq!(a.sheets[0].symbols, b.sheets[0].symbols);
    assert!(preview.at(Point { x_nm: i64::MAX, y_nm: 0 }, 0).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
}

/// PCB duplication cannot silently discard hierarchy-dependent connections between selected
/// parts. A flat subset and already-carried global links remain valid export scopes.
#[test]
fn unsupported_internal_hierarchy_connections_refuse_copy_and_cut_without_loss() {
    use crate::{hierarchy as h, schematic::WireEndpoint};
    let mut s = fixture();
    let root = s.project().unwrap().sheet_target();
    s.apply(h::CREATE_CHILD, &json!({"sheet":root.id(),"name":"Child","x_nm":50*GRID_NM,"y_nm":50*GRID_NM,"expected_revision":s.revision()}))
        .unwrap();
    let child = s.project().unwrap().sheets().find(|p| p.name == "Child").unwrap().target;
    s.apply(h::ADD_LABEL, &json!({"sheet":child.id(),"name":"LINK","direction":"input","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    s.apply(PLACE_SYMBOL, &json!({"sheet":child.id(),"definition":"resistor","x_nm":10*GRID_NM,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let p = s.project().unwrap().for_sheet(child).unwrap();
    let component = p.symbols()[0].component;
    let child_pin = p.symbols()[0].terminals()[0];
    let label = p.labels[0].id;
    let root_pin = s.project().unwrap().symbols()[0].terminals()[0];
    for (sheet, from, to) in [
        (root, WireEndpoint::Terminal(root_pin), WireEndpoint::SheetPin(label)),
        (child, WireEndpoint::Terminal(child_pin), WireEndpoint::Label(label)),
    ] {
        let p = s.project().unwrap().for_sheet(sheet).unwrap();
        let a = p.endpoint_position(from).unwrap();
        let b = p.endpoint_position(to).unwrap();
        let bends = if a.x_nm != b.x_nm && a.y_nm != b.y_nm { vec![Point { x_nm: a.x_nm, y_nm: b.y_nm }] } else { vec![] };
        s.apply(
            crate::DRAW_WIRE,
            &json!({"sheet":sheet.id(),"from":from.inspect(),"to":to.inspect(),"bends":bends,"auto_heal":false,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    s.apply(ASSIGN_FOOTPRINT,&json!({"component":format!("component-{component}"),"library":"starter","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"2","2":"1"},"expected_revision":s.revision()})).unwrap();
    let report = s.execute(crate::PREVIEW_UPDATE, &json!({"expected_revision":s.revision()})).unwrap();
    s.apply(crate::APPLY_UPDATE, &json!({"plan_token":report["board_update_preview"]["plan_token"],"expected_revision":s.revision()})).unwrap();
    s.apply(
        PLACE_ASSIGNED_FOOTPRINT,
        &json!({"component":format!("component-{component}"),"x_nm":20_000_000,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    let before = s.snapshot().unwrap();
    for id in [COPY, CUT] {
        let error = s.execute(id, &json!({"items":all(&s),"expected_revision":s.revision()})).unwrap_err();
        assert!(error.to_string().contains("lose an internal connection"));
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let flat = BoardSelection {
        footprints: view(&s).footprints().filter(|f| f.component != format!("component-{component}")).map(|f| f.id).collect(),
        ..Default::default()
    };
    assert_eq!(BoardFragment::capture(s.project().unwrap(), &flat, false).unwrap().counts().0, 2);
}
