//! Original connector-style fixture dimensions, not taken from a manufacturer drawing.
use super::*;
use crate::{
    ASSIGN_FOOTPRINT, INITIALIZE_BOARD_SETUP, PLACE_ASSIGNED_FOOTPRINT, PLACE_RESISTOR, SET_FOOTPRINT_TRANSFORM, Session,
    authoring::{Action, Item, Kind, Primitive},
    board::{
        clipboard::{BoardFragment, BoardSelection, COPY, PASTE},
        definition::{CopperBinding, PadShape},
        drc::{Kind as Finding, Rules, tests::outline},
        track::tests::view,
    },
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::{Value, json};

/// Construct four separate shell lands and one signal land through the validated editor. A
/// two-terminal placeholder symbol suffices: this exercises mapping, not USB pin assignments.
fn editor() -> crate::authoring::Session {
    let mut editor = crate::authoring::Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    editor.execute(editor.revision(), Action::Delete { objects: vec![1, 3] }).unwrap();
    let Primitive::Pad { pad: template } = editor.snapshot().objects[&2].clone() else { panic!("expected a pad") };
    for (index, (x, y, rotation)) in [(-4, -3, 0), (4, -3, 90), (-4, 3, 90), (4, 3, 0)].into_iter().enumerate() {
        let mut pad = template.clone();
        pad.center = Point { x_nm: x * 1_000_000, y_nm: y * 1_000_000 };
        pad.number = format!("SH{}", index + 1);
        pad.shape = PadShape::Oval;
        pad.width_nm = 1_700_000;
        pad.height_nm = 800_000;
        pad.rotation = rotation;
        pad.corner_radius_nm = 0;
        pad.copper = CopperBinding::AllCopper;
        pad.plated_slot = Some(PlatedSlot::new(560_000, 1_560_000, u32::from(rotation) * 1_000).unwrap());
        editor.execute(editor.revision(), Action::Insert { primitive: Primitive::Pad { pad } }).unwrap();
    }
    editor
}

/// Exercise the ordinary assignment, net annotation, and board placement commands.
fn placed() -> Session {
    let item = editor().snapshot();
    let footprint = item.export_native().unwrap();
    let manifest = json!({"format":crate::library::LIBRARY_FORMAT,"id":item.library,"name":"Synthetic slotted package","license":item.license});
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session
        .execute(
            "library.load_source",
            &json!({"source":"slot-fixture","documents":{
                "library.json":manifest.to_string(),"footprints/slotted.json":footprint.to_string()
            }}),
        )
        .unwrap();
    session.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":session.revision()})).unwrap();
    session.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let project = session.project().unwrap();
    let component = project.components().next().unwrap().id();
    let shell_pin = project.symbols()[0].pins()[0];
    session
        .apply(
            crate::net_labels::ADD,
            &json!({"name":"GND","style":"global","x_nm":shell_pin.x_nm,"y_nm":shell_pin.y_nm,
        "expected_revision":session.revision()}),
        )
        .unwrap();
    session
        .apply(
            ASSIGN_FOOTPRINT,
            &json!({"component":component,"library":item.library,"definition":item.id,
        "definition_revision":item.revision,"mapping":{"1":["SH1","SH2","SH3","SH4"],"2":"2"},"expected_revision":session.revision()}),
        )
        .unwrap();
    session
        .apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":component,"x_nm":10_000_000,"y_nm":10_000_000,"expected_revision":session.revision()}))
        .unwrap();
    outline(&mut session);
    session
}

/// Verify the four independent slots through the same projection used by board painting and
/// routing. The SMD signal land never acquires the shell net or any cut-out.
fn check_placed(session: &Session) {
    use crate::board::view::CopperLayers;
    let footprint = view(session).footprints().next().unwrap();
    assert_eq!(footprint.pads.len(), 5);
    for pad in &footprint.pads {
        if pad.number == "2" {
            assert!(pad.plated_slot.is_none());
            assert_ne!(pad.net.as_deref(), Some("GND"));
            continue;
        }
        let slot = pad.plated_slot.unwrap();
        assert_eq!(pad.drill_nm, None);
        assert_eq!(slot.width_nm(), 560_000);
        let [a, b] = slot.cap_centres();
        assert_eq!((a.x_nm - b.x_nm).abs() + (a.y_nm - b.y_nm).abs(), 1_000_000);
        assert!(matches!(&pad.copper_layers, Some(CopperLayers::Resolved { layers, .. }) if layers.len() == 4));
        assert_eq!(pad.net.as_deref(), Some("GND"));
    }
    let result = view(session).check_design(Rules { min_annular_ring_nm: 70_001, ..Rules::default() });
    assert_eq!(result.counts.get(&Finding::AnnularRing), Some(&4));
    let at_threshold = view(session).check_design(Rules { min_annular_ring_nm: 70_000, ..Rules::default() });
    assert!(!at_threshold.counts.contains_key(&Finding::AnnularRing));
    let inspected = session.inspect();
    let first = &inspected["project"]["board"]["footprints"][0]["pins"];
    let inspected_slots: Vec<_> = first.as_array().unwrap().iter().filter_map(|pin| pin.get("plated_slot")).collect();
    assert_eq!(inspected_slots.len(), 4);
    assert!(inspected_slots.iter().all(|slot| slot["width_nm"] == 560_000 && slot["length_nm"] == 1_560_000));
    match view(session).fabrication() {
        Ok(_) => panic!("slot machining was silently omitted"),
        Err(error) => assert!(error.contains("cannot yet machine plated slots")),
    }
}

/// A synthetic four-slot footprint retains authored geometry, named net and annular-ring
/// checks after reorientation, clipboard transport, and native reopen.
#[test]
fn four_shell_slots_keep_geometry_and_ground_through_native_commands() {
    let mut session = placed();
    check_placed(&session);
    let footprint = view(&session).footprints().next().unwrap();
    let id = footprint.id;
    session
        .apply(
            SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":id,"x_nm":10_000_000,"y_nm":10_000_000,"side":"back","rotation":90,
        "expected_revision":session.revision()}),
        )
        .unwrap();
    check_placed(&session);
    let before = session.snapshot().unwrap();
    let documents = encode_documents(before.clone()).unwrap();
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    assert_eq!(encode_documents(decode_packed(&encode_packed(before.clone()).unwrap()).unwrap().project).unwrap(), documents);
    let selection = BoardSelection { footprints: [id].into(), ..Default::default() };
    let copied = session.execute(COPY, &json!({"items":selection,"expected_revision":session.revision()})).unwrap();
    let text = copied["clipboard"].as_str().unwrap();
    let fragment: Value = serde_json::from_str(text).unwrap();
    assert_eq!(fragment["format"], "pcbcraft-clipboard/8");
    let mut disguised = fragment.clone();
    disguised["format"] = json!("pcbcraft-clipboard/7");
    assert!(matches!(BoardFragment::from_text(&disguised.to_string()),
        Err(CommandError::InvalidParams(message)) if message.contains("format does not match its pinned footprint holes")));
    let circuit = fragment["circuits"][0]["text"].as_str().unwrap();
    let mut circuit: Value = serde_json::from_str(circuit).unwrap();
    assert_eq!(circuit["format"], "pcbcraft-clipboard/7");
    circuit["format"] = json!("pcbcraft-clipboard/6");
    assert!(matches!(crate::clipboard::Fragment::from_text(&circuit.to_string()),
        Err(CommandError::InvalidParams(message)) if message.contains("format does not match its pinned footprint holes")));
    let sheet = session.project().unwrap().sheet_target().id().to_string();
    session
        .apply(
            PASTE,
            &json!({"text":text,"x_nm":20_000_000,"y_nm":20_000_000,"rotation":90,
        "schematic_targets":{(sheet.clone()):{"sheet":sheet,"at":{"x_nm":0,"y_nm":30 * crate::schematic::GRID_NM}}},
        "expected_revision":session.revision()}),
        )
        .unwrap();
    let after = session.snapshot().unwrap();
    let views: Vec<_> = view(&session).footprints().collect();
    assert_eq!(views.len(), 2);
    assert!(views.iter().all(|footprint| footprint.pads.iter().filter(|pad| pad.plated_slot.is_some()).count() == 4));
    session.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &session.snapshot().unwrap());
    session.apply("edit.redo", &Value::Null).unwrap();
    crate::assert_same_content(&after, &session.snapshot().unwrap());
}

/// Front and back tracks contacting opposite ends of one shell land join its plated island.
/// The other shell lands retain the same intended GND without magically becoming routed.
#[test]
fn a_slot_connects_copper_through_the_stack() {
    use crate::board::track::tests::{copper_layers, route};
    let mut session = placed();
    let footprint = view(&session).footprints().next().unwrap();
    let key = (footprint.id, "SH1".to_owned());
    let pad = footprint.pads.iter().find(|pad| pad.number == "SH1").unwrap();
    let at = |x| Point { x_nm: pad.center.x_nm + x, y_nm: pad.center.y_nm };
    let layers = copper_layers(&session);
    let ends = [[at(-2_000_000), at(-800_000)], [at(2_000_000), at(800_000)]];
    for (layer, points) in layers.into_iter().zip(ends) {
        route(&mut session, layer, &points).unwrap();
    }
    let board = view(&session);
    let (copper, nets) = board.copper_with_nets();
    assert_eq!(board.tracks().len(), 2);
    for track in board.tracks() {
        assert_eq!(copper.tracks[&track.id], copper.pads[&key]);
        assert_eq!(nets[&track.id], "GND");
    }
    assert_ne!(copper.pads[&key], copper.pads[&(key.0.clone(), "SH2".into())]);
}

/// The editable and published versions are selected lazily, and malformed slot edits
/// refuse the complete replacement without adding undo history.
#[test]
fn slot_authoring_keeps_versions_lazy_and_refuses_breakout_atomically() {
    let mut editor = editor();
    let snapshot = editor.snapshot();
    let bytes = snapshot.encode().unwrap();
    assert_eq!(Item::decode(&bytes).unwrap().encode().unwrap(), bytes);
    let mut item: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(item["format"], "pcbcraft-library-item/4");
    item["format"] = json!("pcbcraft-library-item/1");
    assert_eq!(Item::decode(&serde_json::to_vec(&item).unwrap()).unwrap_err(), "Invalid or oversized native library item.");
    let Primitive::Pad { mut pad } = snapshot.objects[&4].clone() else { panic!("expected shell pad") };
    let unchanged = editor.revision();
    pad.plated_slot = Some(PlatedSlot::new(560_000, 1_560_000, 90_000).unwrap());
    assert_eq!(
        editor.execute(unchanged, Action::Replace { object: 4, primitive: Primitive::Pad { pad } }).unwrap_err(),
        "invalid footprint identity, metadata, pad identities or rectangle geometry"
    );
    assert_eq!(editor.revision(), unchanged);
    assert_eq!(editor.snapshot(), snapshot);
    editor.execute(unchanged, Action::Delete { objects: vec![4, 5, 6, 7] }).unwrap();
    assert_eq!(editor.snapshot().export_native().unwrap()["format"], "pcbcraft-footprint/1");
    assert_eq!(serde_json::to_value(editor.snapshot().as_ref()).unwrap()["format"], "pcbcraft-library-item/1");
    editor.execute(editor.revision(), Action::Undo).unwrap();
    assert_eq!(editor.snapshot().export_native().unwrap()["format"], "pcbcraft-footprint/5");
}

/// Combined drafts and definitions prefer the slot versions, without dropping circular NPTH
/// when the last slot is removed or the published footprint is opened for editing.
#[test]
fn mixed_footprint_preserves_both_hole_types_and_downgrades_lazily() {
    let mut editor = editor();
    let hole = crate::board::definition::CircularNpth { center: Point { x_nm: -6_000_000, y_nm: 0 }, diameter_nm: 500_000 };
    editor.execute(editor.revision(), Action::Insert { primitive: Primitive::NonPlatedHole { hole } }).unwrap();
    let item = editor.snapshot();
    let footprint = item.footprint().unwrap();
    assert_eq!(footprint.format, "pcbcraft-footprint/5");
    assert_eq!(footprint.non_plated_holes, [hole]);
    let reopened = Item::edit_footprint(&footprint).unwrap();
    assert_eq!(serde_json::to_value(&reopened).unwrap()["format"], "pcbcraft-library-item/4");
    assert_eq!(reopened.footprint().unwrap().pads, footprint.pads);
    editor.execute(editor.revision(), Action::Delete { objects: vec![4, 5, 6, 7] }).unwrap();
    assert_eq!(editor.snapshot().footprint().unwrap().format, "pcbcraft-footprint/4");
    assert_eq!(serde_json::to_value(editor.snapshot().as_ref()).unwrap()["format"], "pcbcraft-library-item/3");
    assert_eq!(editor.snapshot().footprint().unwrap().non_plated_holes, [hole]);
}
