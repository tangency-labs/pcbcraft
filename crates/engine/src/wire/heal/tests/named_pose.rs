//! Properties and R preserve explicit naming contacts, including custom offset terminals.
use super::*;

/// Create an offset naming terminal whose initial contact has no attached endpoint unless
/// requested explicitly. Fixtures use the native source loader and shared commands.
fn fixture(contact: &str) -> (Session, String, WireEndpoint, Vec<WireEndpoint>) {
    let mut s = session();
    let (_, mut docs) = crate::library::bundled_documents().find(|(id, _)| *id == "power").unwrap();
    let mut manifest: Value = serde_json::from_str(&docs["library.json"]).unwrap();
    manifest["id"] = json!("custom-rails");
    docs.insert("library.json".into(), manifest.to_string());
    let mut definition: Value = serde_json::from_str(&docs["symbols/gnd.json"]).unwrap();
    definition["units"][0]["pins"][0]["at_nm"] = json!([p(2, 0).x_nm, 0]);
    docs.retain(|path, _| path == "library.json");
    docs.insert("symbols/gnd.json".into(), definition.to_string());
    s.execute(crate::library::sources::LOAD, &json!({"source":"custom","documents":docs})).unwrap();
    let mut contacts = Vec::new();
    if contact == "pin" {
        run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":p(6,0).x_nm,"y_nm":0}));
        contacts.push(WireEndpoint::Terminal(s.project().unwrap().symbols()[0].terminals()[0]));
    }
    let definition = s.libraries().latest_symbol("custom-rails", "gnd").unwrap();
    run(
        &mut s,
        PLACE_SYMBOL,
        json!({"library":"custom-rails","definition":"gnd","definition_revision":definition.revision(),
            "definition_digest":crate::library::sources::symbol_digest(&definition).unwrap(),"x_nm":0,"y_nm":0}),
    );
    let symbol = s.project().unwrap().symbols().last().unwrap().clone();
    let named = WireEndpoint::Terminal(symbol.terminals()[0]);
    if contact == "interior" {
        draw(&mut s, p(-8, 0), p(8, 0));
        contacts.extend(s.project().unwrap().wires()[0].endpoints());
    } else if contact == "explicit" {
        run(&mut s, DRAW_WIRE, json!({"from":named.inspect(),"to":p(12,8),"bends":[p(2,4),p(12,4)]}));
        contacts.push(s.project().unwrap().wires()[0].endpoints()[1]);
    }
    (s, symbol.id(), named, contacts)
}

/// Every pose entry point keeps the named terminal joined to fixed pins and wire interiors
/// in both cleanup modes; explicitly attached routes retain their identities and rotation bends.
#[test]
fn named_contacts_survive_properties_and_rotation() {
    for contact in ["pin", "interior", "explicit"] {
        for edit in ["fields_move", "fields_rotate", "rotate"] {
            for auto_heal in [false, true] {
                let (mut s, symbol, named, contacts) = fixture(contact);
                let before = s.snapshot().unwrap();
                let graph = before.electrical_connectivity();
                let original_net = graph.net_index_for_endpoint(named).unwrap();
                for end in &contacts {
                    assert_eq!(graph.net_index_for_endpoint(*end), Some(original_net));
                }
                if edit == "rotate" {
                    run(&mut s, crate::ROTATE_SYMBOL, json!({"symbol":symbol}));
                } else {
                    let component = before.component(before.symbols().last().unwrap()).unwrap();
                    let mut params = json!({"symbol":symbol,"reference":component.reference(),"value":component.value(),"auto_heal":auto_heal});
                    if edit == "fields_move" {
                        params["x_nm"] = json!(p(0, 6).x_nm);
                        params["y_nm"] = json!(p(0, 6).y_nm);
                    } else {
                        params["rotation"] = json!(90);
                    }
                    run(&mut s, crate::SET_SYMBOL_FIELDS, params);
                }
                let after = s.snapshot().unwrap();
                assert_ne!(before.endpoint_position(named), after.endpoint_position(named));
                let graph = after.electrical_connectivity();
                let net = graph.net_index_for_endpoint(named).unwrap();
                assert_eq!(graph.net_name(net), "GND");
                for end in &contacts {
                    assert_eq!(after.endpoint_position(*end), before.endpoint_position(*end));
                    assert_eq!(graph.net_index_for_endpoint(*end), Some(net), "{contact}, {edit}, auto_heal={auto_heal}");
                }
                if contact == "explicit" && edit != "fields_move" {
                    assert_eq!(after.wires(), before.wires(), "Rotation must retain explicit endpoints and authored bends");
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
}
