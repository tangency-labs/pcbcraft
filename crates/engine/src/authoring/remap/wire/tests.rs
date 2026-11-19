use super::*;
use crate::authoring::{Item, Kind, remap};

/// Adding remap discovery keeps every native source command reachable through the shared catalog.
#[test]
fn pin_remap_discovery_preserves_native_source_commands() {
    for source in crate::library::sources::COMMANDS {
        assert_eq!(crate::command(source.id).map(|command| command.id), Some(source.id), "{} is missing from discovery", source.id);
    }
}

/// Each scalar parser accepts its full native string range and refuses numeric JSON tokens.
/// Emitted cases permit independent validation of the production schemas, not a schema stand-in.
#[test]
fn pin_remap_wire_domains_are_exact_decimal_strings() {
    for kind in [Scalar::Object, Scalar::BaseRevision, Scalar::Runtime, Scalar::Token] {
        let (min, max) = kind.bounds();
        let mut cases = vec![
            (json!(min.to_string()), true),
            (json!(max.to_string()), true),
            (json!((max - 1).to_string()), true),
            (json!((u128::from(max) + 1).to_string()), false),
            (json!("0"), min == 0),
        ];
        for number in [1, (1u64 << 53) + 1, i64::MAX as u64 - 1, i64::MAX as u64, u64::MAX] {
            cases.push((json!(number.to_string()), number <= max));
            cases.push((json!(number), false));
        }
        for bad in ["", "00", "01", "-0", "-1", "+1", " 1", "1 ", "1\n", "1.0", "1e0", "１", "١"] {
            cases.push((json!(bad), false));
        }
        for raw in [
            "null",
            "true",
            "{}",
            "[]",
            "0",
            "-0",
            "1.0",
            "1e0",
            "1.0000000000000001",
            "9007199254740993.0",
            "18446744073709551615.0",
            "18446744073709551616",
        ] {
            cases.push((serde_json::from_str(raw).unwrap(), false));
        }
        for (value, valid) in cases {
            let read = kind.parse(&value);
            assert_eq!(read.is_ok(), valid, "{kind:?}: {value}: {read:?}");
            if let Err(reason) = read {
                assert!(reason.contains("canonical decimal string"), "{reason}");
            }
            eprintln!("pin_remap_schema_case {}", json!({"schema":kind.schema(),"value":value,"reader_accepts":valid}));
        }
    }
    let decoded: Value = serde_json::from_str(r#""\u0031""#).unwrap();
    assert_eq!(Scalar::Object.parse(&decoded).unwrap(), 1);
}

/// Published-revision bounds and omission are checked through the actual headless command.
/// Reports stringify only remap fields; the embedded authoring item retains its native codec.
#[test]
fn pin_remap_headless_revision_and_report_wire_match() {
    let item = Item::new(Kind::Symbol).unwrap();
    let mut session = crate::Session::default();
    let command = crate::command(crate::library::sources::PREVIEW_PIN_REMAP).unwrap();
    let params = json!({"item":item,"pins":[{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}]});
    for revision in [None, Some("1"), Some("4294967295")] {
        let mut request = params.clone();
        if let Some(revision) = revision {
            request["base_revision"] = json!(revision);
        }
        let before = session.inspect();
        let report = session.execute(command.id, &request).unwrap();
        assert_eq!(report["source_revision"], json!(revision));
        assert_eq!(report["changes"][0]["object"], "1");
        assert!(report["item"]["revision"].is_number());
        assert_eq!(session.inspect(), before);
        eprintln!("pin_remap_schema_case {}", json!({"schema":command.to_json()["params"],"value":request,"reader_accepts":true}));
    }
    for value in [Value::Null, json!(1), json!(1.0), json!("0"), json!("4294967296"), json!("1.0"), json!("1e0"), json!("01")] {
        let mut request = params.clone();
        request["base_revision"] = value;
        let before = session.inspect();
        let error = session.execute(command.id, &request).unwrap_err();
        let crate::CommandError::InvalidParams(message) = error else { panic!("wrong error kind: {error}") };
        assert!(message.contains("base_revision") && message.contains("canonical decimal string"), "{message}");
        assert_eq!(session.inspect(), before);
        eprintln!("pin_remap_schema_case {}", json!({"schema":command.to_json()["params"],"value":request,"reader_accepts":false}));
    }
    assert_eq!(remap::decode(&params["pins"]).unwrap()[0].object, 1);
}

/// Structural schemas cover presence, spelling and exact repeated rows. Distinct rows reusing
/// an identity/destination and an opaque invalid item are refused by the checked reader instead.
/// Emit both the structural expectation and the actual semantic refusal for external validation.
#[test]
fn pin_remap_headless_shape_and_semantic_refusals_are_distinct() {
    let command = crate::command(crate::library::sources::PREVIEW_PIN_REMAP).unwrap();
    let params =
        json!({"item":Item::new(Kind::Symbol).unwrap(),"pins":[{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}]});
    let mut cases = Vec::new();
    for (field, reason) in [("item", "`library.preview_pin_remap` needs `item`"), ("pins", "`library.preview_pin_remap` needs `pins`")] {
        let mut request = params.clone();
        request.as_object_mut().unwrap().remove(field);
        cases.push((request, false, reason));
    }
    let keys = "`pins` Each pin remap row requires only unit, object and number.";
    for field in ["unit", "object", "number"] {
        let mut request = params.clone();
        request["pins"][0].as_object_mut().unwrap().remove(field);
        cases.push((request, false, keys));
    }
    let mut unknown = params.clone();
    unknown["pins"][0]["guess"] = json!(true);
    cases.push((unknown, false, keys));
    let mut empty = params.clone();
    empty["pins"] = json!([]);
    cases.push((empty, false, "`pins` Choose 1 to 128 logical pins to remap."));
    let mut large = params.clone();
    for count in [1, 128] {
        large["pins"] = (1..=count).map(|id| json!({"unit":"main","object":id.to_string(),"number":format!("n{id}")})).collect();
        // This checks the production parameter reader; the two-pin native item cannot resolve 128 rows.
        let accepted = crate::check_params(command, &large).is_ok();
        assert!(accepted, "{large}");
        eprintln!("pin_remap_schema_case {}", json!({"schema":command.to_json()["params"],"value":large,"reader_accepts":accepted}));
    }
    large["pins"].as_array_mut().unwrap().push(json!({"unit":"main","object":"129","number":"n129"}));
    cases.push((large, false, "`pins` Choose 1 to 128 logical pins to remap."));
    let mut exact = params.clone();
    exact["pins"][1] = exact["pins"][0].clone();
    cases.push((exact, false, "`pins` Unknown, duplicate or invalid pin identity."));
    let mut identity = params.clone();
    identity["pins"][1]["object"] = json!("1");
    cases.push((identity, true, "`pins` Unknown, duplicate or invalid pin identity."));
    let mut destination = params.clone();
    destination["pins"][1]["number"] = json!("2");
    cases.push((destination, true, "`pins` Invalid or duplicate destination terminal number."));
    for row in [Value::Null, json!(false), json!(["main", "1", "2"])] {
        let mut malformed = params.clone();
        malformed["pins"][0] = row;
        cases.push((malformed, false, "`pins` Each pin remap row must be an object."));
    }
    for item in [Value::Null, json!([]), json!(false)] {
        let mut request = params.clone();
        request["item"] = item;
        cases.push((request, false, "`item` Pin remap item must be an object."));
    }
    let mut opaque = params;
    opaque["item"] = json!({});
    cases.push((opaque, true, "missing field `format` at line 1 column 2"));
    let mut session = crate::Session::default();
    for (request, expected_schema_accepts, reason) in cases {
        let before = session.inspect();
        let error = session.execute(command.id, &request).unwrap_err();
        let crate::CommandError::InvalidParams(message) = error else { panic!("wrong error kind: {error}") };
        assert_eq!(message, reason, "{request}");
        assert_eq!(session.inspect(), before);
        assert!(session.snapshot().is_none());
        eprintln!(
            "pin_remap_schema_case {}",
            json!({"schema":command.to_json()["params"],"value":request,"reader_accepts":false,"reader_refusal":message,"expected_schema_accepts":expected_schema_accepts})
        );
    }
}
