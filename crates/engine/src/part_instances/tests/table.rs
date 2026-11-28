//! The component table's rows: one per physical component, read without touching history.
use super::fields::{fixture, footprint, run};
use crate::{Session, SessionScope};
use serde_json::{Value, json};

/// The read command under test, spelled as clients send it.
const INSPECT_FIELDS: &str = "part.inspect_fields";

/// Five resistors and a two-unit amplifier read as six rows in project order. The amplifier is
/// one row counting both units; assigned packages and component fields appear on their rows.
#[test]
fn rows_list_each_physical_component_once_with_its_fields_and_footprint() {
    let mut s = fixture();
    let ids: Vec<String> = s.project().unwrap().components().map(|part| part.id()).collect();
    let maker = json!({"manufacturer":{"name":"Manufacturer","value":"Example Resistors"}});
    run(&mut s, json!([{"component":ids[0],"footprint":footprint(json!({"1":"1","2":"2"}))},{"component":ids[1],"fields":maker}]), false).unwrap();
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let reply = s.execute(INSPECT_FIELDS, &json!({})).unwrap();
    assert_eq!(s.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(reply["revision"], json!(revision));
    let rows = reply["components"].as_array().unwrap();
    assert_eq!(rows.iter().map(|row| row["component"].as_str().unwrap()).collect::<Vec<_>>(), ids);
    assert_eq!(rows.iter().map(|row| row["units"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 1, 1, 1, 1, 2]);
    assert_eq!(rows[0]["footprint"]["library"], "basic-components");
    assert_eq!(rows[0]["footprint"]["definition"], "resistor-0402");
    assert_eq!(rows[0]["footprint"]["definition_revision"], 4);
    assert!(rows[0]["footprint"]["name"].as_str().is_some_and(|name| !name.is_empty()), "{}", rows[0]);
    assert_eq!(rows[1]["fields"]["manufacturer"]["value"], "Example Resistors");
    assert_eq!(rows[1]["footprint"], Value::Null);
    assert_eq!(rows[0]["reference"], "R1");
    assert_eq!(json!(s.project().unwrap().component_rows()), reply["components"]);
}

/// A read-only session lists the same rows; without a project the read is refused.
#[test]
fn read_only_sessions_read_the_rows_and_no_project_is_refused() {
    let s = fixture();
    let mut reader = Session::with_scope(SessionScope::ReadOnly);
    reader.open_snapshot(s.snapshot().unwrap());
    assert_eq!(reader.execute(INSPECT_FIELDS, &json!({})).unwrap()["components"].as_array().unwrap().len(), 6);
    assert!(Session::default().execute(INSPECT_FIELDS, &json!({})).is_err());
    assert!(reader.execute(INSPECT_FIELDS, &json!({"unknown":true})).is_err());
}
