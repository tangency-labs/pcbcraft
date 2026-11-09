use super::*;

/// Place a native naming terminal with the ordinary placement command and exact source digest.
fn power(s: &mut Session, at: Point) -> WireEndpoint {
    let d = s.libraries().latest_symbol("power", "gnd").unwrap();
    run(
        s,
        PLACE_SYMBOL,
        json!({"library":"power","definition":"gnd","definition_revision":d.revision(),"definition_digest":crate::library::sources::symbol_digest(&d).unwrap(),"x_nm":at.x_nm,"y_nm":at.y_nm}),
    );
    WireEndpoint::Terminal(s.project().unwrap().symbols().last().unwrap().terminals()[0])
}

/// One original resistor pin with an explicitly placed ground marker at the same coordinate.
fn marked_pin() -> (Session, WireEndpoint, WireEndpoint) {
    let mut s = session();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(14,8).x_nm,"y_nm":p(14,8).y_nm}));
    let pin = WireEndpoint::Terminal(s.project().unwrap().symbols()[0].terminals()[0]);
    let label = power(&mut s, p(10, 8));
    (s, pin, label)
}

/// Automatic whole-wire drops treat a pin and its power marker as one contact in either
/// cleanup mode, preserving marker identity, named connectivity, native bytes and history.
#[test]
fn wire_drops_onto_a_marked_pin_are_one_contact() {
    for auto_heal in [false, true] {
        let (mut s, pin, label) = marked_pin();
        draw(&mut s, p(0, 0), p(10, 0));
        let wire = s.project().unwrap().wires[0].id();
        let before = s.snapshot().unwrap();
        run(&mut s, MOVE_ITEMS, json!({"items":{"symbols":[],"wires":[wire]},"dx_nm":0,"dy_nm":p(0,8).y_nm,"auto_heal":auto_heal}));
        let project = s.project().unwrap();
        assert_eq!(project.wires[0].endpoints[1], pin);
        assert_eq!(project.endpoint_position(label), Some(p(10, 8)));
        if let WireEndpoint::Terminal(pin) = pin {
            assert_eq!(project.connectivity().net_name(project.connectivity().net_index_for(pin).unwrap()), "GND");
        }
        let documents = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        s.apply("edit.redo", &Value::Null).unwrap();
        assert_eq!(s.project().unwrap().wires[0].endpoints[1], pin);
    }
}

/// Explicit end drops retain the identity already used by the target route. A named marker
/// declares the contact even when target routes use distinct pin/marker endpoint identities.
#[test]
fn explicit_marked_contacts_preserve_existing_route_endpoints() {
    for auto_heal in [false, true] {
        for target_count in [0, 1, 2] {
            for label_first in [false, true] {
                let (mut s, pin, label) = marked_pin();
                let targets = if label_first { [(label, p(10, 16)), (pin, p(0, 8))] } else { [(pin, p(0, 8)), (label, p(10, 16))] };
                for (end, to) in targets.into_iter().take(target_count) {
                    run(&mut s, DRAW_WIRE, json!({"from":end.inspect(),"to":to,"bends":[]}));
                }
                let original: Vec<_> = s.project().unwrap().wires.iter().map(|w| (w.id(), w.endpoints)).collect();
                draw(&mut s, p(10, -8), p(10, 0));
                let source = s.project().unwrap().wires.last().unwrap().clone();
                let junction = source.endpoints[1].inspect()["junction"].clone();
                let before = s.snapshot().unwrap();
                run(
                    &mut s,
                    crate::MOVE_WIRE_SEGMENT,
                    json!({"wire":source.id(),"at":p(10,-4),"to":p(10,4),"connect_ends":[junction],"auto_heal":auto_heal}),
                );
                let project = s.project().unwrap();
                let expected = original.first().map_or(pin, |(_, ends)| ends[0]);
                assert_eq!(project.wires.iter().find(|w| w.id() == source.id()).unwrap().endpoints[1], expected);
                for (id, endpoints) in original {
                    assert_eq!(project.wires.iter().find(|w| w.id() == id).unwrap().endpoints, endpoints);
                }
                let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
                assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
                s.apply("edit.undo", &Value::Null).unwrap();
                assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
            }
        }
    }
}

/// Without a marker declaring the contact, distinct component pins at the same coordinate
/// remain ambiguous, and neither automatic nor explicit connection can partially edit history.
#[test]
fn unmarked_coincident_pins_remain_atomic_refusals() {
    for explicit in [false, true] {
        for auto_heal in [false, true] {
            let mut s = session();
            for x in [6, 14] {
                run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(x,8).x_nm,"y_nm":p(x,8).y_nm}));
            }
            draw(&mut s, p(0, 0), p(10, 0));
            let wire = s.project().unwrap().wires[0].clone();
            let junction = wire.endpoints[1].inspect()["junction"].clone();
            let before = s.snapshot().unwrap();
            let revision = s.revision();
            let (command, params) = if explicit {
                (
                    crate::MOVE_WIRE_SEGMENT,
                    json!({"wire":wire.id(),"at":p(5,0),"to":p(5,8),"connect_ends":[junction],"auto_heal":auto_heal,"expected_revision":revision}),
                )
            } else {
                (
                    MOVE_ITEMS,
                    json!({"items":{"symbols":[],"wires":[wire.id()]},"dx_nm":0,"dy_nm":p(0,8).y_nm,"auto_heal":auto_heal,"expected_revision":revision}),
                )
            };
            assert!(s.apply(command, &params).unwrap_err().to_string().contains("coincident"));
            assert_eq!(s.revision(), revision);
            assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        }
    }
}

/// Standalone markers and markers deliberately joining several coincident pins are explicit
/// contacts. The marker identity avoids choosing an arbitrary component pin in either case.
#[test]
fn declared_contacts_without_one_unique_component_pin_keep_the_marker_identity() {
    for physical_pins in [0, 2] {
        for auto_heal in [false, true] {
            let mut s = session();
            if physical_pins == 2 {
                for x in [6, 14] {
                    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(x,8).x_nm,"y_nm":p(x,8).y_nm}));
                }
            }
            let marker = power(&mut s, p(10, 8));
            draw(&mut s, p(0, 0), p(10, 0));
            let wire = s.project().unwrap().wires.last().unwrap().id();
            run(&mut s, MOVE_ITEMS, json!({"items":{"symbols":[],"wires":[wire]},"dx_nm":0,"dy_nm":p(0,8).y_nm,"auto_heal":auto_heal}));
            let project = s.project().unwrap();
            assert_eq!(project.wires.iter().find(|w| w.id() == wire).unwrap().endpoints[1], marker);
            for pin in project.symbols().iter().flat_map(|s| s.terminals()).filter(|pin| project.terminal_position(*pin) == Some(p(10, 8))) {
                assert_eq!(project.connectivity().net_name(project.connectivity().net_index_for(pin).unwrap()), "GND");
            }
        }
    }
}

/// A marker already connects an interior route to its coincident pin. An explicit drop can
/// use that contact without splitting or changing either endpoint of the existing route.
#[test]
fn marked_interior_contacts_preserve_the_existing_wire() {
    for auto_heal in [false, true] {
        let (mut s, pin, _) = marked_pin();
        draw(&mut s, p(0, 8), p(20, 8));
        let target = s.project().unwrap().wires[0].clone();
        draw(&mut s, p(10, -8), p(10, 0));
        let source = s.project().unwrap().wires.last().unwrap().clone();
        let junction = source.endpoints[1].inspect()["junction"].clone();
        run(&mut s, crate::MOVE_WIRE_SEGMENT, json!({"wire":source.id(),"at":p(10,-4),"to":p(10,4),"connect_ends":[junction],"auto_heal":auto_heal}));
        let project = s.project().unwrap();
        assert_eq!(project.wires.len(), 2);
        assert_eq!(project.wires.iter().find(|w| w.id() == target.id()).unwrap(), &target);
        assert_eq!(project.wires.iter().find(|w| w.id() == source.id()).unwrap().endpoints[1], pin);
    }
}
