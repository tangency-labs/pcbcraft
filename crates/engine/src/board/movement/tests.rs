use super::*;
use crate::{
    ADD_DRAWING, ADD_TRACKS, ADD_VIA, CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_RESISTOR,
    persistence::{decode_documents, encode_documents},
};
use serde_json::json;

/// Submit through the same revision-checked catalog as GUI and live/headless clients.
fn edit(s: &mut Session, id: &str, mut p: Value) -> Result<u64, CommandError> {
    p["expected_revision"] = json!(s.revision());
    s.apply(id, &p)
}
/// Two placed two-net footprints, an outline, zone and free copper exercise every category.
fn fixture() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    for x in [0, 25_400_000] {
        edit(&mut s, PLACE_RESISTOR, json!({"x_nm":x,"y_nm":0})).unwrap();
    }
    for t in ["1", "2"] {
        edit(&mut s, CONNECT_PINS, json!({"from":{"symbol":"symbol-2","terminal":t},"to":{"symbol":"symbol-3","terminal":t}})).unwrap();
    }
    edit(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true})).unwrap();
    for (symbol, x) in [("symbol-2", 0), ("symbol-3", 25_400_000)] {
        edit(&mut s, PLACE_FOOTPRINT, json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":x,"y_nm":0})).unwrap();
    }
    let p = s.project().unwrap();
    let board = p.board_view(p.board_target().unwrap()).unwrap();
    let layer = board.layers().into_iter().find(|l| l.purpose == super::super::setup::LayerPurpose::Copper).unwrap().id;
    let outline = board.layers().into_iter().find(|l| l.purpose == super::super::setup::LayerPurpose::Outline).unwrap().id;
    let net = p.inspect_net_registry().unwrap()["nets"][0]["id"].clone();
    edit(
        &mut s,
        super::super::zones::ADD_ZONE,
        json!({"layer":layer,"net":net,"points":[{"x_nm":0,"y_nm":10_000_000},{"x_nm":10_000_000,"y_nm":10_000_000},{"x_nm":0,"y_nm":20_000_000}]}),
    )
    .unwrap();
    edit(
        &mut s,
        ADD_DRAWING,
        json!({"layer":outline,"closed":false,"points":[{"x_nm":-10_000_000,"y_nm":30_000_000},{"x_nm":10_000_000,"y_nm":30_000_000}]}),
    )
    .unwrap();
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":40_000_000},{"x_nm":10_000_000,"y_nm":40_000_000}]}))
        .unwrap();
    edit(&mut s, ADD_VIA, json!({"at":{"x_nm":10_000_000,"y_nm":40_000_000}})).unwrap();
    s
}
/// Collect model identities rather than correspondence highlights.
fn all(s: &Session) -> BoardSelection {
    let p = s.project().unwrap();
    let b = p.board_view(p.board_target().unwrap()).unwrap();
    BoardSelection {
        footprints: b.footprint_ids().collect(),
        drawings: b.drawings().iter().map(|d| d.id).collect(),
        zones: b.zones().iter().map(|z| z.id).collect(),
        tracks: b.tracks().iter().map(|t| t.id).collect(),
        vias: b.vias().iter().map(|v| v.id).collect(),
    }
}
/// Preview immutability, all-category identity/translation, no-op and exact native history agree.
#[test]
fn translation_is_atomic_and_reopens_exactly() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let selection = all(&s);
    let delta = Point { x_nm: 3_000_000, y_nm: -2_000_000 };
    let plan = MovePlan::new(before.clone(), before.board_target().unwrap(), selection.clone()).unwrap();
    let shown = plan.at(delta).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let revision = s.revision();
    edit(&mut s, MOVE_ITEMS, json!({"items":selection,"dx_nm":delta.x_nm,"dy_nm":delta.y_nm})).unwrap();
    assert_eq!(s.revision(), revision + 1);
    let after = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(encode_documents(shown).unwrap(), after);
    assert_eq!(encode_documents(decode_documents(&after).unwrap()).unwrap(), after);
    edit(&mut s, MOVE_ITEMS, json!({"items":selection,"dx_nm":0,"dy_nm":0})).unwrap();
    assert_eq!(s.revision(), revision + 1);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), after);
}
/// Unknown IDs, stale revisions, bounds and locked objects cannot move a subset or create history.
#[test]
fn refused_groups_leave_all_objects_and_history_unchanged() {
    let mut s = fixture();
    let selection = all(&s);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let mut missing = selection.clone();
    missing.zones.insert(Uuid::nil());
    for p in [
        json!({"items":missing,"dx_nm":1000,"dy_nm":0,"expected_revision":revision}),
        json!({"items":selection,"dx_nm":1000,"dy_nm":0,"expected_revision":revision-1}),
        json!({"items":selection,"dx_nm":crate::schematic::MAX_CENTER_NM,"dy_nm":0,"expected_revision":revision}),
    ] {
        assert!(s.apply(MOVE_ITEMS, &p).is_err());
        assert_eq!(s.revision(), revision);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let target = s.project().unwrap().board_target().unwrap();
    edit(&mut s, crate::SET_COPPER_LOCKED, json!({"board":target.board,"tracks":selection.tracks,"locked":true})).unwrap();
    let locked = s.snapshot().unwrap();
    assert!(edit(&mut s, MOVE_ITEMS, json!({"items":selection,"dx_nm":1000,"dy_nm":0})).is_err());
    assert!(Arc::ptr_eq(&locked, &s.snapshot().unwrap()));
}
/// A pad-end carry moves only the attached end of unselected copper and never shifts selected copper twice.
#[test]
fn captured_pad_contacts_and_selected_tracks_translate_once() {
    let mut s = fixture();
    let p = s.project().unwrap();
    let b = p.board_view(p.board_target().unwrap()).unwrap();
    let f = b.footprints().next().unwrap();
    let fp = f.id.clone();
    let at = f.pads[0].center;
    let Some(super::super::view::CopperLayers::Resolved { layers, .. }) = &f.pads[0].copper_layers else { panic!("configured pad") };
    let layer = layers[0];
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[at,{"x_nm":at.x_nm,"y_nm":at.y_nm-10_000_000}]})).unwrap();
    let p = s.project().unwrap();
    let b = p.board_view(p.board_target().unwrap()).unwrap();
    let attached = b.tracks().iter().find(|t| t.start == at).unwrap().clone();
    for include in [false, true] {
        let selection = BoardSelection {
            footprints: [fp.clone()].into(),
            tracks: if include { [attached.id].into() } else { BTreeSet::new() },
            ..Default::default()
        };
        edit(&mut s, MOVE_ITEMS, json!({"items":selection,"dx_nm":1_000_000,"dy_nm":0})).unwrap();
        let moved = s.project().unwrap().board.tracks.iter().find(|t| t.id == attached.id).unwrap();
        assert_eq!(moved.start.x_nm, attached.start.x_nm + 1_000_000);
        assert_eq!(moved.end.x_nm, attached.end.x_nm + if include { 1_000_000 } else { 0 });
        s.apply("edit.undo", &Value::Null).unwrap();
    }
}

/// A moved pad cannot short to a different net on an unselected footprint, even when the same
/// transaction also moves a zone. Refusal retains every category and the exact history root.
#[test]
fn a_physical_short_refuses_the_entire_mixed_group() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let b = before.board_view(before.board_target().unwrap()).unwrap();
    let f: Vec<_> = b.footprints().collect();
    let a = f[0].pads.iter().find(|p| p.number == "1").unwrap().center;
    let z = f[1].pads.iter().find(|p| p.number == "2").unwrap().center;
    let selection = BoardSelection { footprints: [f[0].id.clone()].into(), zones: b.zones().iter().map(|z| z.id).collect(), ..Default::default() };
    assert!(edit(&mut s, MOVE_ITEMS, json!({"items":selection,"dx_nm":z.x_nm-a.x_nm,"dy_nm":z.y_nm-a.y_nm})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}
