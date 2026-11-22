//! Appearance overrides follow electrical networks, including junction-only wire geometry.
use super::*;
use crate::{Terminal, WireEndpoint};

/// A junction-to-junction wire and a junction-to-terminal stub inherit the override of the
/// network they electrically join, even though junctions are not logical graph anchors.
#[test]
fn appearance_follows_junction_only_wires_through_electrical_network() {
    let (mut s, pins) = circuit(3, false);
    let base = connect(&mut s, &pins[0], &pins[1]);
    let at = s.project().unwrap().terminal_position(Terminal::parse(&pins[1]).unwrap()).unwrap();
    apply(&mut s, crate::DRAW_WIRE, json!({"from":pins[1],"to":{"x_nm":at.x_nm,"y_nm":at.y_nm+12_700_000},"bends":[],"auto_heal":false}));
    let stub = s.project().unwrap().wires().last().unwrap().id();
    let junction = s.project().unwrap().wires().last().unwrap().endpoints()[1];
    assert!(matches!(junction, WireEndpoint::Junction(_)));

    let junction_at = s.project().unwrap().endpoint_position(junction).unwrap();
    apply(
        &mut s,
        crate::DRAW_WIRE,
        json!({"from":junction.inspect(),"to":{"x_nm":junction_at.x_nm,"y_nm":junction_at.y_nm+12_700_000},"bends":[],"auto_heal":false}),
    );
    let bridge = s.project().unwrap().wires().last().unwrap().id();
    let ends = s.project().unwrap().wires().last().unwrap().endpoints();
    assert!(matches!(ends[0], WireEndpoint::Junction(_)) && matches!(ends[1], WireEndpoint::Junction(_)));

    let id = promote(&mut s, &pins[0]);
    let project = s.project().unwrap();
    let target = serde_json::to_value(ClassTarget::Persistent { project: project.id(), scope: project.net_scope(), net: id }).unwrap();
    let expected = Appearance { color: Some([9, 8, 7]), width_cp: Some(200) };
    apply(&mut s, SET_APPEARANCE, json!({"target":target,"appearance":serde_json::to_value(expected).unwrap()}));

    let wires = s.project().unwrap().wire_appearances().unwrap();
    assert_eq!(wires.get(&base), Some(&expected));
    assert_eq!(wires.get(&stub), Some(&expected));
    assert_eq!(wires.get(&bridge), Some(&expected));
}

/// Storage and command decoders distinguish an omitted optional field from an explicit null.
#[test]
fn appearance_fields_reject_explicit_null_but_allow_omission() {
    assert!(serde_json::from_value::<Appearance>(json!({"color":null,"width_cp":100})).is_err());
    assert!(serde_json::from_value::<Appearance>(json!({"color":[1,2,3],"width_cp":null})).is_err());
    assert!(serde_json::from_value::<Appearance>(json!({"width_cp":100})).is_ok());
    assert!(serde_json::from_value::<Appearance>(json!({"color":[1,2,3]})).is_ok());

    let (mut s, pins) = circuit(2, false);
    connect(&mut s, &pins[0], &pins[1]);
    let id = promote(&mut s, &pins[0]);
    let project = s.project().unwrap();
    let target = serde_json::to_value(ClassTarget::Persistent { project: project.id(), scope: project.net_scope(), net: id }).unwrap();
    let revision = s.revision();
    for patch in [
        json!({"color":[1,2,3],"width_cp":null}),
        json!({"color":null,"width_cp":100}),
        json!({"clear":null}),
        json!({"clear":[]}),
        json!({"clear":["unknown"]}),
        json!({"unknown":1}),
        json!({"width_cp":49}),
        json!({"color":[1,2,3],"clear":["color"]}),
    ] {
        assert!(s.apply(SET_APPEARANCE, &json!({"target":target,"appearance":patch,"expected_revision":revision})).is_err());
    }
    assert_eq!(s.revision(), revision);
    assert_eq!(s.project().unwrap().net_registry.record(id).unwrap().appearance, None);
}

/// A clear marker removes one saved channel while the other is inherited, in one command and one
/// undo step. Omitting a channel keeps it; explicit values set it; null and {} are full resets.
#[test]
fn appearance_clear_marker_removes_one_channel_and_is_one_undo_step() {
    let (mut s, pins) = circuit(2, false);
    connect(&mut s, &pins[0], &pins[1]);
    let id = promote(&mut s, &pins[0]);
    let project = s.project().unwrap();
    let target = serde_json::to_value(ClassTarget::Persistent { project: project.id(), scope: project.net_scope(), net: id }).unwrap();
    let both = Appearance { color: Some([1, 2, 3]), width_cp: Some(100) };
    apply(&mut s, SET_APPEARANCE, json!({"target":target,"appearance":serde_json::to_value(both).unwrap()}));

    for (clear, expected) in
        [("width_cp", Appearance { color: Some([1, 2, 3]), width_cp: None }), ("color", Appearance { color: None, width_cp: Some(100) })]
    {
        let revision = s.revision();
        apply(&mut s, SET_APPEARANCE, json!({"target":target,"appearance":{"clear":[clear]}}));
        assert_eq!(s.revision(), revision + 1);
        assert_eq!(s.project().unwrap().net_registry.record(id).unwrap().appearance, Some(expected));
        apply(&mut s, "edit.undo", json!({}));
        assert_eq!(s.project().unwrap().net_registry.record(id).unwrap().appearance, Some(both));
    }

    // An omitted channel inherits the saved value; an explicit value replaces only that channel.
    apply(&mut s, SET_APPEARANCE, json!({"target":target,"appearance":{"color":[4,5,6]}}));
    assert_eq!(s.project().unwrap().net_registry.record(id).unwrap().appearance, Some(Appearance { color: Some([4, 5, 6]), width_cp: Some(100) }));

    // Top-level null and an empty object are full resets.
    apply(&mut s, SET_APPEARANCE, json!({"target":target,"appearance":null}));
    assert_eq!(s.project().unwrap().net_registry.record(id).unwrap().appearance, None);
    apply(&mut s, SET_APPEARANCE, json!({"target":target,"appearance":{"color":[1,2,3]}}));
    apply(&mut s, SET_APPEARANCE, json!({"target":target,"appearance":{}}));
    assert_eq!(s.project().unwrap().net_registry.record(id).unwrap().appearance, None);
}

/// The published patch schema advertises exactly the clear channels the command accepts, while
/// the saved record schema stays free of command-only markers.
#[test]
fn appearance_patch_schema_advertises_supported_clear_channels() {
    assert!(Appearance::schema()["properties"].get("clear").is_none());

    let clear = &crate::nets::appearance_param_schema()["oneOf"][1]["properties"]["clear"];
    assert_eq!(clear["type"], "array");
    assert_eq!(clear["items"]["enum"], json!(["color", "width_cp"]));

    for channel in ["color", "width_cp"] {
        assert!(crate::nets::validate_appearance_patch(&json!({"clear":[channel]})).is_ok());
    }
    assert!(crate::nets::validate_appearance_patch(&json!({"clear":[]})).is_err());
    assert!(crate::nets::validate_appearance_patch(&json!({"clear":["unknown"]})).is_err());
}
