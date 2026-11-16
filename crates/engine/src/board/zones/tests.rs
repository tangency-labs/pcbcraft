use super::*;
use crate::{
    CONNECT_PINS, INITIALIZE_BOARD_SETUP, PLACE_RESISTOR,
    persistence::{decode_documents, encode_documents},
};
/// Build meaningful unnamed nets through production commands, with explicit physical ownership.
fn fixture() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    for x in [0, 25_400_000, 50_800_000] {
        edit(&mut s, PLACE_RESISTOR, json!({"x_nm":x,"y_nm":0})).unwrap();
    }
    edit(&mut s, CONNECT_PINS, json!({"from":{"symbol":"symbol-2","terminal":"1"},"to":{"symbol":"symbol-3","terminal":"1"}})).unwrap();
    edit(&mut s, INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true})).unwrap();
    s
}
/// Commands share the same revision precondition as GUI and MCP.
fn edit(s: &mut Session, id: &str, mut params: Value) -> Result<u64, CommandError> {
    params["expected_revision"] = json!(s.revision());
    s.apply(id, &params)
}
fn points() -> Vec<Point> {
    vec![
        Point { x_nm: 0, y_nm: 0 },
        Point { x_nm: 10_000_000, y_nm: 0 },
        Point { x_nm: 10_000_000, y_nm: 10_000_000 },
        Point { x_nm: 0, y_nm: 10_000_000 },
    ]
}
fn add_params(s: &Session) -> Value {
    let p = s.project().unwrap();
    let board = p.board_view(p.board_target().unwrap()).unwrap();
    let layer = board.setup().unwrap().layers().iter().find(|l| l.purpose == super::super::setup::LayerPurpose::Copper).unwrap().id;
    let nets = p.inspect_net_registry().unwrap();
    json!({"layer":layer,"points":points(),"target":{"kind":"derived","anchor":nets["nets"][0]["anchor"]}})
}
/// Identity promotion and creation are one undo step, without conductive copper or fill.
#[test]
fn creation_promotion_codec_and_history_are_atomic() {
    let mut s = fixture();
    let before = s.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let revision = s.revision();
    let params = add_params(&s);
    edit(&mut s, ADD_ZONE, params).unwrap();
    assert_eq!(s.revision(), revision + 1);
    let p = s.snapshot().unwrap();
    assert_eq!(p.board.zones.len(), 1);
    let zone = &p.board.zones[0];
    assert_eq!(p.zone_net_state(zone.net), "active");
    assert!(p.board.tracks.is_empty());
    assert!(p.board.vias.is_empty());
    assert!(before.net_registry.record(zone.net).is_none());
    let encoded = encode_documents(p.clone()).unwrap();
    let board_path = p.board_document_path().unwrap();
    let doc: Value = serde_json::from_slice(&encoded[board_path.as_str()]).unwrap();
    assert_eq!(doc["format"], "pcbcraft-board/20");
    assert_eq!(doc["zones"].as_array().unwrap().len(), 1);
    assert!(doc.get("fill").is_none());
    let reopened = decode_documents(&encoded).unwrap();
    assert_eq!(encode_documents(reopened).unwrap(), encoded);
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), encoded);
    let id = s.project().unwrap().board.zones[0].id;
    edit(&mut s, DELETE_ZONES, json!({"zones":[id]})).unwrap();
    let remaining = encode_documents(s.snapshot().unwrap()).unwrap();
    let doc: Value = serde_json::from_slice(&remaining[board_path.as_str()]).unwrap();
    assert_ne!(doc["format"], "pcbcraft-board/20");
    assert!(doc.get("zones").is_none());
}
/// Malformed/foreign net, layer, geometry and settings never publish even a promoted registry.
#[test]
fn rejected_zones_preserve_everything() {
    let mut s = fixture();
    let original = s.snapshot().unwrap();
    let valid = add_params(&s);
    let mut cases = Vec::new();
    for (key, value) in [
        ("points", json!([{"x_nm":0,"y_nm":0},{"x_nm":10,"y_nm":10},{"x_nm":0,"y_nm":10},{"x_nm":10,"y_nm":0}])),
        ("layer", json!(crate::fresh_id().unwrap())),
        ("points", json!([{"x_nm":0,"y_nm":0},{"x_nm":1,"y_nm":1}])),
    ] {
        let mut bad = valid.clone();
        bad[key] = value;
        cases.push(bad);
    }
    let mut bad = valid.clone();
    bad.as_object_mut().unwrap().remove("target");
    bad["net"] = json!(crate::fresh_id().unwrap());
    cases.push(bad);
    let mut bad = valid.clone();
    bad["options"] = json!(Options { thermal_gap_nm: 0, ..Default::default() });
    cases.push(bad);
    let mut bad = valid.clone();
    bad["target"]["anchor"]["project"] = json!(crate::fresh_id().unwrap());
    cases.push(bad);
    for bad in cases {
        let rev = s.revision();
        assert!(edit(&mut s, ADD_ZONE, bad.clone()).is_err(), "{bad}");
        assert_eq!(s.revision(), rev);
        assert!(Arc::ptr_eq(&original, &s.snapshot().unwrap()));
    }
    let mut stale = valid;
    stale["expected_revision"] = json!(s.revision() + 1);
    assert!(s.apply(ADD_ZONE, &stale).is_err());
    assert!(Arc::ptr_eq(&original, &s.snapshot().unwrap()));
}
/// Setting an existing zone preserves its UUID and no-ops; malformed storage is refused.
#[test]
fn editing_and_refused_storage() {
    let mut s = fixture();
    let params = add_params(&s);
    edit(&mut s, ADD_ZONE, params).unwrap();
    let original = s.snapshot().unwrap();
    let z = original.board.zones[0].clone();
    edit(&mut s, SET_ZONE, json!({"zone":z.id,"points":z.points})).unwrap();
    assert!(Arc::ptr_eq(&original, &s.snapshot().unwrap()));
    let options = Options { priority: 42, clearance_nm: Some(250_000), connection: Connection::Solid, ..Default::default() };
    edit(&mut s, SET_ZONE, json!({"zone":z.id,"options":options})).unwrap();
    assert_eq!(s.project().unwrap().board.zones[0].options, options);
    let project = s.snapshot().unwrap();
    let docs = encode_documents(project.clone()).unwrap();
    let path = project.board_document_path().unwrap();
    for value in [Value::Null, json!([])] {
        let mut forged = docs.clone();
        let mut board: Value = serde_json::from_slice(&forged[path.as_str()]).unwrap();
        board["zones"] = value;
        forged.insert(path.clone(), serde_json::to_vec(&board).unwrap());
        assert!(decode_documents(&forged).is_err());
    }
    let mut forged = docs;
    let mut board: Value = serde_json::from_slice(&forged[path.as_str()]).unwrap();
    board["format"] = json!("pcbcraft-board/19");
    forged.insert(path, serde_json::to_vec(&board).unwrap());
    assert!(decode_documents(&forged).is_err());
}

/// Zone format retains locked physical copper and earlier board intent on a byte-stable reopen.
#[test]
fn zones_round_trip_with_locked_copper() {
    let mut s = fixture();
    let params = add_params(&s);
    let layer = params["layer"].clone();
    edit(&mut s, ADD_ZONE, params).unwrap();
    edit(&mut s, crate::board::track::ADD_TRACKS, json!({"layer":layer,"width_nm":250_000,"points":[points()[0],points()[1]]})).unwrap();
    let track = s.project().unwrap().board.tracks[0].id;
    let board = s.project().unwrap().board_target().unwrap().board;
    edit(&mut s, crate::board::track::SET_COPPER_LOCKED, json!({"board":board,"tracks":[track],"locked":true})).unwrap();
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert!(reopened.board.tracks[0].locked);
    assert_eq!(reopened.board.zones.len(), 1);
    assert_eq!(encode_documents(reopened).unwrap(), docs);
}
/// Exact integer checks allow concavity and large valid coordinates without admitting crossings.
#[test]
fn outline_validates_simple_bounded_polygons() {
    assert!(validate_outline(&points()).is_ok());
    let mut reverse = points();
    reverse.reverse();
    assert!(validate_outline(&reverse).is_ok());
    assert!(validate_outline(&[Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 1, y_nm: 1 }, Point { x_nm: 2, y_nm: 2 }]).is_err());
    let mut closed = points();
    closed.push(closed[0]);
    assert!(validate_outline(&closed).is_err());
    let max = crate::schematic::MAX_CENTER_NM;
    assert!(
        validate_outline(&[
            Point { x_nm: -max, y_nm: -max },
            Point { x_nm: max, y_nm: -max },
            Point { x_nm: max, y_nm: max },
            Point { x_nm: -max, y_nm: max }
        ])
        .is_ok()
    );
}
/// Anchored splits preserve the zone's branch, while merges retain a visible retired target.
#[test]
fn zone_identity_follows_anchored_split_and_reports_merge() {
    let mut s = fixture();
    edit(&mut s, PLACE_RESISTOR, json!({"x_nm":75_000_000,"y_nm":0})).unwrap();
    let pins: Vec<_> = s.project().unwrap().symbols().iter().map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"})).collect();
    edit(&mut s, CONNECT_PINS, json!({"from":pins[1],"to":pins[2]})).unwrap();
    let middle = s.project().unwrap().wires().last().unwrap().id();
    edit(&mut s, CONNECT_PINS, json!({"from":pins[2],"to":pins[3]})).unwrap();
    let params = add_params(&s);
    edit(&mut s, ADD_ZONE, params).unwrap();
    let original = s.project().unwrap().board.zones[0].net;
    edit(&mut s, crate::DELETE_WIRE, json!({"wire":middle})).unwrap();
    assert_eq!(s.project().unwrap().board.zones[0].net, original);
    assert_eq!(s.project().unwrap().zone_net_state(original), "active");
    assert!(s.project().unwrap().net_registry.records.iter().any(|r| r.id != original && matches!(r.state, State::Active {})));
    edit(&mut s, CONNECT_PINS, json!({"from":pins[1],"to":pins[2]})).unwrap();
    let p = s.snapshot().unwrap();
    assert_eq!(p.board.zones[0].net, original);
    assert_eq!(p.zone_net_state(original), "retired");
    assert_eq!(p.inspect_zones()[0]["net_state"], "retired");
    let docs = encode_documents(p).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert_eq!(reopened.zone_net_state(original), "retired");
    assert_eq!(encode_documents(reopened).unwrap(), docs);
}
/// Losing every stored anchor exposes ambiguity on the zone instead of choosing either branch.
#[test]
fn ambiguous_zone_target_remains_explicit_and_undoable() {
    let mut s = fixture();
    let params = add_params(&s);
    edit(&mut s, ADD_ZONE, params).unwrap();
    let net = s.project().unwrap().board.zones[0].net;
    for x in [75_000_000, 100_000_000, 125_000_000] {
        edit(&mut s, PLACE_RESISTOR, json!({"x_nm":x,"y_nm":0})).unwrap();
    }
    let pins: Vec<_> = s.project().unwrap().symbols().iter().map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"})).collect();
    for (from, to) in [(0, 2), (2, 3), (0, 4), (4, 5)] {
        edit(&mut s, CONNECT_PINS, json!({"from":pins[from],"to":pins[to]})).unwrap();
    }
    let selection = crate::selection::Selection {
        symbols: [pins[0]["symbol"].as_str().unwrap().into(), pins[1]["symbol"].as_str().unwrap().into()].into(),
        ..Default::default()
    };
    edit(&mut s, crate::DELETE_ITEMS, json!({"items":selection})).unwrap();
    let p = s.snapshot().unwrap();
    assert_eq!(p.board.zones[0].net, net);
    assert_eq!(p.zone_net_state(net), "ambiguous");
    let docs = encode_documents(p).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert_eq!(reopened.zone_net_state(net), "ambiguous");
    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.project().unwrap().zone_net_state(net), "active");
    s.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), docs);
}

/// The cap is enforced on the transaction boundary, preserving the last accepted snapshot.
#[test]
fn zone_count_and_codec_references_are_bounded() {
    let mut s = fixture();
    let mut params = add_params(&s);
    params["points"] = json!([{"x_nm":0,"y_nm":0},{"x_nm":1000000,"y_nm":0},{"x_nm":0,"y_nm":1000000}]);
    for _ in 0..MAX_ZONES {
        edit(&mut s, ADD_ZONE, params.clone()).unwrap();
    }
    let full = s.snapshot().unwrap();
    assert_eq!(full.boards().next().unwrap().zones().len(), MAX_ZONES);
    let revision = s.revision();
    assert!(edit(&mut s, ADD_ZONE, params).is_err());
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&full, &s.snapshot().unwrap()));
    let docs = encode_documents(full.clone()).unwrap();
    let path = full.board_document_path().unwrap();
    for field in ["net", "layer"] {
        let mut changed = docs.clone();
        let mut board: Value = serde_json::from_slice(&changed[path.as_str()]).unwrap();
        board["zones"][0][field] = json!(crate::fresh_id().unwrap());
        changed.insert(path.clone(), serde_json::to_vec(&board).unwrap());
        assert!(decode_documents(&changed).is_err());
    }
}
