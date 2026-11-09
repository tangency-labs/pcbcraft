use super::*;
use pcbcraft_engine::{SET_SYMBOL_FIELDS, persistence::encode_documents, selection::Selection};

/// The live endpoint exposes optional bounded coordinates and produces the same atomic edit
/// as the inspector. Invalid input leaves both the project and its history unchanged.
#[test]
fn live_property_coordinates_match_the_inspector() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    h.state_mut().properties_open = true;
    h.state_mut().select(Selection::one_symbol("symbol-2".into()));
    h.run();
    let (port, token) = connect(&mut h);
    let connection = (port, token.as_str());
    let schema = payload(tool(&mut h, connection, "describe_command", json!({"command":SET_SYMBOL_FIELDS})));
    for axis in ["x_nm", "y_nm"] {
        assert_eq!(schema["params"]["properties"][axis]["type"], "integer");
        assert!(!schema["params"]["required"].as_array().unwrap().contains(&json!(axis)));
    }
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    for (label, text) in [("X", "1 mm"), ("Y", "-25 mil")] {
        h.get_by_label(label).click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.get_by_label(label).type_text(text);
        h.run();
    }
    click(&mut h, "Apply changes");
    assert_eq!(h.state().session.revision(), revision + 1);
    let gui = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    payload(tool(&mut h, connection, "execute", json!({"command":"edit.undo","params":{}})));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let revision = h.state().session.revision();
    let params = json!({"symbol":"symbol-2","reference":"R1","value":"R","rotation":0,
        "x_nm":1_000_000,"y_nm":-635_000,"expected_revision":revision});
    payload(tool(&mut h, connection, "execute", json!({"command":SET_SYMBOL_FIELDS,"params":params})));
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), gui);
    let before = h.state().session.inspect();
    let refused = tool(
        &mut h,
        connection,
        "execute",
        json!({"command":SET_SYMBOL_FIELDS,"params":{
            "symbol":"symbol-2","reference":"R1","value":"not applied","x_nm":"1 mm","expected_revision":revision+1
        }}),
    );
    assert_eq!(refused["isError"], true);
    assert_eq!(h.state().session.inspect(), before);
}
