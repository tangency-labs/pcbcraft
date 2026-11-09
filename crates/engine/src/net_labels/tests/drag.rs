//! Moving a marker preserves exact source contacts regardless of how they were authored.
use super::*;

/// Build each source contact using engine commands, with the marker placed last.
fn fixture(kind: &str) -> (Session, u64, Point) {
    let mut s = session();
    let mut origin = Point { x_nm: 0, y_nm: 0 };
    if kind == "pin" {
        apply(&mut s, PLACE_RESISTOR, json!({"x_nm":-20*G,"y_nm":0}));
        origin = s.project().unwrap().symbols()[0].pins()[1];
    } else if kind == "end" {
        apply(&mut s, PLACE_RESISTOR, json!({"x_nm":-20*G,"y_nm":0}));
        let pin = s.project().unwrap().symbols()[0].terminals()[1];
        apply(&mut s, DRAW_WIRE, json!({"from":pin.inspect(),"to":origin,"bends":[]}));
    } else if kind == "interior" || kind == "crossing" {
        apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":-10*G,"y_nm":0},"to":{"x_nm":10*G,"y_nm":0},"bends":[]}));
        if kind == "crossing" {
            apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":-10*G},"to":{"x_nm":0,"y_nm":10*G},"bends":[]}));
        }
    } else if kind == "branch" {
        apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":-10*G,"y_nm":0},"to":origin,"bends":[]}));
        let end = s.project().unwrap().wires()[0].endpoints()[1];
        apply(&mut s, DRAW_WIRE, json!({"from":end.inspect(),"to":{"x_nm":0,"y_nm":-10*G},"bends":[]}));
    } else if kind == "named" {
        apply(&mut s, ADD, json!({"name":"LOCAL","style":"local","x_nm":0,"y_nm":0}));
    }
    apply(&mut s, ADD, json!({"name":"GND","style":"ground","x_nm":origin.x_nm,"y_nm":origin.y_nm}));
    let id = s.project().unwrap().net_labels().last().unwrap().id;
    (s, id, origin)
}

/// Free ends, branches, interiors, crossings and fixed pins/markers remain connected during
/// preview and commit; unaffected anchors stay fixed. All allocation and geometry undo once.
#[test]
fn connected_marker_moves_capture_contacts_and_match_the_preview() {
    for kind in ["end", "interior", "branch", "crossing", "pin", "named", "free"] {
        for auto_heal in [false, true] {
            let (mut s, id, origin) = fixture(kind);
            let before = s.snapshot().unwrap();
            let to = Point { x_nm: origin.x_nm + 6 * G, y_nm: origin.y_nm + 8 * G };
            let preview = before.dragged_net_label(id, to, 90, s.revision(), auto_heal).unwrap();
            assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
            let end = WireEndpoint::NetLabel(id);
            let anchors: BTreeSet<_> = before
                .wires()
                .iter()
                .flat_map(|wire| wire.endpoints())
                .chain(before.symbols().iter().flat_map(|symbol| symbol.terminals().into_iter().map(WireEndpoint::Terminal)))
                .chain(before.hierarchy_endpoints().map(|(end, _)| end))
                .filter(|other| *other != end && (!matches!(other, WireEndpoint::Junction(_)) || before.endpoint_position(*other) != Some(origin)))
                .collect();
            let old_graph = before.electrical_connectivity();
            apply(
                &mut s,
                SET,
                json!({"id":format!("net-label-{id}"),"name":"GND","style":"ground","x_nm":to.x_nm,"y_nm":to.y_nm,"rotation":90,"keep_connected":true,"auto_heal":auto_heal}),
            );
            let after = s.snapshot().unwrap();
            crate::assert_same_content(&Arc::new(preview), &after);
            assert_eq!(after.endpoint_position(end), Some(to));
            let graph = after.electrical_connectivity();
            let old_net = old_graph.net_index_for_endpoint(end).unwrap();
            let new_net = graph.net_index_for_endpoint(end).unwrap();
            for other in anchors {
                assert_eq!(after.endpoint_position(other), before.endpoint_position(other), "{kind}");
                assert_eq!(old_graph.net_index_for_endpoint(other) == Some(old_net), graph.net_index_for_endpoint(other) == Some(new_net), "{kind}");
            }
            if kind != "free" {
                assert!(after.wires().iter().any(|wire| wire.endpoints().contains(&end)), "{kind}");
            }
            let saved = persistence::encode_documents(after.clone()).unwrap();
            assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
            s.apply("edit.undo", &Value::Null).unwrap();
            crate::assert_same_content(&before, &s.snapshot().unwrap());
            s.apply("edit.redo", &Value::Null).unwrap();
            crate::assert_same_content(&after, &s.snapshot().unwrap());
        }
    }
}

/// A snapshot with a populated named-net cache must resolve the new destination in either
/// preview mode, while retaining the old source graph unchanged.
#[test]
fn named_contact_caches_follow_both_preview_modes() {
    let (mut s, id, _) = fixture("end");
    let to = Point { x_nm: 6 * G, y_nm: 8 * G };
    apply(&mut s, DRAW_WIRE, json!({"from":to,"to":{"x_nm":16*G,"y_nm":8*G},"bends":[]}));
    let before = s.snapshot().unwrap();
    let end = WireEndpoint::NetLabel(id);
    let pin = WireEndpoint::Terminal(before.symbols()[0].terminals()[1]);
    let destination = before.wires().last().unwrap().endpoints()[1];
    let old = before.electrical_connectivity();
    assert_ne!(old.net_index_for_endpoint(end), old.net_index_for_endpoint(destination));
    for connected in [false, true] {
        let preview =
            if connected { before.dragged_net_label(id, to, 0, s.revision(), true).unwrap() } else { before.preview_net_label(id, to, 0).unwrap() };
        let graph = preview.electrical_connectivity();
        let net = graph.net_index_for_endpoint(end).unwrap();
        assert_eq!(graph.net_index_for_endpoint(destination), Some(net));
        assert_eq!(graph.net_index_for_endpoint(pin) == Some(net), connected);
    }
    assert_eq!(before.electrical_connectivity().net_index_for_endpoint(end), old.net_index_for_endpoint(end));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// Invalid or stale connected moves never consume history or publish split/merge work.
/// Existing SET callers keep their default geometry-only behavior unless they request it.
#[test]
fn marker_contact_capture_is_optional_and_rejected_edits_are_atomic() {
    let (mut s, id, _) = fixture("interior");
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    for (name, rotation, expected, keep) in [
        (" bad", 0, revision, json!(true)),
        ("GND", 45, revision, json!(true)),
        ("GND", 0, revision - 1, json!(true)),
        ("GND", 0, revision, json!("yes")),
    ] {
        assert!(s.apply(SET, &json!({"id":format!("net-label-{id}"),"name":name,"style":"ground","x_nm":6*G,"y_nm":8*G,"rotation":rotation,"keep_connected":keep,"expected_revision":expected})).is_err());
        assert_eq!(s.revision(), revision);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    apply(&mut s, SET, json!({"id":format!("net-label-{id}"),"name":"GND","style":"ground","x_nm":6*G,"y_nm":8*G}));
    let after = s.snapshot().unwrap();
    assert_eq!(after.wires(), before.wires());
    assert_eq!(after.wire_path(&after.wires()[0]), before.wire_path(&before.wires()[0]));
    assert_eq!(crate::command(SET).unwrap().to_json()["params"]["properties"]["keep_connected"]["default"], false);
}
