//! One bulk edit over physical components: validated as a whole, previewed, one undo step.
use crate::{
    CommandError, PLACE_RESISTOR, PLACE_SYMBOL, PLACE_UNIT, SET_SYMBOL_FIELDS, Session,
    persistence::{decode_documents, encode_documents},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// The command under test, spelled as clients send it.
const SET_FIELDS: &str = "part.set_fields";
const DUAL: &str = include_str!("../../../../../examples/native-parts/symbols/dual-amplifier.json");
const CUSTOM: &str = "6f9a2a3c-0c6b-4a55-9d2e-2b1d7f0c1a11";

/// Run a setup command on the ordinary revision-checked path.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Five resistors and one two-unit amplifier with both units placed, without a board.
fn fixture() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    for i in 0..5 {
        apply(&mut s, PLACE_RESISTOR, json!({"x_nm": i * 10_160_000, "y_nm": 0}));
    }
    s.execute(
        "library.load_source",
        &json!({"source":"units","documents":{
            "library.json":r#"{"format":"pcbcraft-library/1","id":"units","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
            "symbols/dual.json":DUAL,
        }}),
    )
    .unwrap();
    let definition = s.libraries.symbol("units", "dual-amplifier", 1).unwrap();
    let digest = crate::library::sources::symbol_digest(&definition).unwrap();
    apply(
        &mut s,
        PLACE_SYMBOL,
        json!({"library":"units","definition":"dual-amplifier","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":25_400_000}),
    );
    let part = s.project().unwrap().symbols().last().unwrap().component_id();
    apply(&mut s, PLACE_UNIT, json!({"component":part,"unit":"b","x_nm":25_400_000,"y_nm":25_400_000}));
    s
}

/// Submit one bulk edit against the current revision.
fn run(s: &mut Session, edits: Value, dry_run: bool) -> Result<Value, CommandError> {
    let params = json!({"edits":edits,"dry_run":dry_run,"expected_revision":s.revision()});
    s.execute(SET_FIELDS, &params)
}

/// Inspected component rows, in project order.
fn rows(s: &Session) -> Vec<Value> {
    s.inspect()["project"]["components"].as_array().unwrap().clone()
}

/// A complete two-pad footprint choice for a basic resistor.
fn footprint(mapping: Value) -> Value {
    json!({"library":"basic-components","definition":"resistor-0402","definition_revision":4,"mapping":mapping})
}

/// One field set across three parts and one value is one undo step; every other field, and
/// every component not named, keeps its content.
#[test]
fn one_field_across_several_parts_is_one_undo_step_and_keeps_other_fields() {
    let mut s = fixture();
    let start = rows(&s);
    let symbol = s.project().unwrap().symbols()[0].id();
    let attributes = json!({"do_not_populate":false,"exclude_from_bom":false,"exclude_from_board":false,
        "exclude_from_simulation":false,"exclude_from_position_files":false});
    apply(
        &mut s,
        SET_SYMBOL_FIELDS,
        json!({"symbol":symbol,"reference":start[0]["reference"],"value":start[0]["value"],
            "metadata":{"fields":{CUSTOM:{"name":"Tolerance","value":"1%"}},"attributes":attributes}}),
    );
    let start = rows(&s);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let maker = json!({"manufacturer":{"name":"Manufacturer","value":"Example Resistors"}});
    let edits = json!([
        {"component":start[0]["id"],"fields":maker},
        {"component":start[1]["id"],"fields":maker,"value":"4.7k"},
        {"component":start[2]["id"],"fields":maker},
    ]);
    let reply = run(&mut s, edits, false).unwrap();
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(reply["revision"], json!(s.revision()));
    assert_eq!(reply["changes"].as_array().unwrap().len(), 4, "{reply}");
    let after = rows(&s);
    for row in &after[..3] {
        assert_eq!(row["metadata"]["fields"]["manufacturer"]["value"], "Example Resistors");
    }
    assert_eq!(after[0]["metadata"]["fields"][CUSTOM], json!({"name":"Tolerance","value":"1%"}));
    assert_eq!(after[0]["value"], start[0]["value"]);
    assert_eq!(after[1]["value"], "4.7k");
    assert_eq!(after[3..], start[3..]);
    s.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    assert_eq!(rows(&s), start);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(rows(&s), after);
}

/// A part with two placed units is one row: its units share the edited component, and a unit's
/// placement ID is not accepted as a row.
#[test]
fn a_multi_unit_part_is_one_row_and_every_unit_follows() {
    let mut s = fixture();
    let amplifier = rows(&s).last().unwrap()["id"].as_str().unwrap().to_owned();
    let units: Vec<_> = s.project().unwrap().symbols().iter().filter(|x| x.component_id() == amplifier).map(|x| x.id()).collect();
    assert_eq!(units.len(), 2);
    let datasheet = json!({"datasheet":{"name":"Datasheet","value":"https://example.com/dual.pdf"}});
    let reply = run(&mut s, json!([{"component":amplifier,"reference":"U7","value":"Dual op-amp","fields":datasheet}]), false).unwrap();
    assert_eq!(reply["changes"].as_array().unwrap().len(), 3, "{reply}");
    let project = s.project().unwrap();
    assert_eq!(project.components().count(), 6);
    for symbol in project.symbols().iter().filter(|x| x.component_id() == amplifier) {
        let part = project.component(symbol).unwrap();
        assert_eq!((part.reference(), part.value()), ("U7", "Dual op-amp"));
        assert_eq!(part.metadata.fields["datasheet"].value, "https://example.com/dual.pdf");
    }
    let revision = s.revision();
    let error = run(&mut s, json!([{"component":units[0],"value":"10k"}]), false).unwrap_err();
    assert!(matches!(error, CommandError::InvalidParams(_)), "{error:?}");
    assert_eq!(s.revision(), revision);
}

/// Any invalid row refuses every row, in a dry run too, leaving revision, snapshot and saved
/// bytes untouched. Uniqueness is judged on the complete result, so a swap is accepted.
#[test]
fn an_invalid_row_refuses_the_whole_edit_and_changes_nothing() {
    let mut s = fixture();
    let start = rows(&s);
    let good = json!({"component":start[0]["id"],"value":"1k"});
    let cases = [
        json!([good, {"component":start[1]["id"],"reference":start[0]["reference"]}]),
        json!([good, {"component":start[1]["id"],"reference":"R 2"}]),
        json!([good, {"component":"component-999","value":"1k"}]),
        json!([good, {"component":start[0]["id"],"value":"2k"}]),
        json!([good, {"component":start[1]["id"],"fields":{"tolerance":{"name":"Tolerance","value":"1%"}}}]),
        json!([good, {"component":start[1]["id"],"fields":{"manufacturer":{"name":"Value","value":"x"}}}]),
        json!([good, {"component":start[1]["id"],"footprint":footprint(json!({"1":"1"}))}]),
        json!([good, {"component":start[1]["id"],"footprint":{"library":"basic-components","definition":"missing","definition_revision":1,"mapping":{"1":"1","2":"2"}}}]),
        json!([good, {"component":start[1]["id"],"colour":"red"}]),
    ];
    for edits in cases {
        let before = s.snapshot().unwrap();
        let documents = encode_documents(before.clone()).unwrap();
        let revision = s.revision();
        assert!(run(&mut s, edits.clone(), true).is_err(), "dry run accepted {edits}");
        assert!(run(&mut s, edits.clone(), false).is_err(), "accepted {edits}");
        assert_eq!(s.revision(), revision);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), documents);
    }
    let swap = json!([{"component":start[0]["id"],"reference":start[1]["reference"]},{"component":start[1]["id"],"reference":start[0]["reference"]}]);
    run(&mut s, swap, false).unwrap();
    let after = rows(&s);
    assert_eq!((&after[0]["reference"], &after[1]["reference"]), (&start[1]["reference"], &start[0]["reference"]));
}

/// The dry run reports exactly the field changes Apply then makes, and changes neither revision,
/// history nor saved bytes. Unchanged named values are not listed. Undo and Redo cover the footprint.
#[test]
fn dry_run_lists_exactly_what_apply_then_changes() {
    let mut s = fixture();
    let start = rows(&s);
    let edits = json!([
        {"component":start[0]["id"],"value":"10k","fields":{"manufacturer_part_number":{"name":"Manufacturer part number","value":"EX-10K"}}},
        {"component":start[4]["id"],"reference":"R10","value":start[4]["value"]},
        {"component":start[2]["id"],"footprint":footprint(json!({"1":"1","2":"2"}))},
    ]);
    let before = s.snapshot().unwrap();
    let documents = encode_documents(before.clone()).unwrap();
    let revision = s.revision();
    let preview = run(&mut s, edits.clone(), true).unwrap();
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), documents);
    let row = |i: usize, field: &str, old: Value, new: Value| json!({"component":start[i]["id"],"reference":start[i]["reference"],"field":field,"old":old,"new":new});
    let expected = json!([
        row(0, "value", start[0]["value"].clone(), json!("10k")),
        row(0, "manufacturer_part_number", Value::Null, json!({"name":"Manufacturer part number","value":"EX-10K"})),
        row(4, "reference", start[4]["reference"].clone(), json!("R10")),
        row(2, "footprint", Value::Null, footprint(json!({"1":"1","2":"2"}))),
    ]);
    assert_eq!(preview, json!({"revision":revision,"changes":expected}));
    let applied = run(&mut s, edits, false).unwrap();
    assert_eq!(applied, json!({"revision":revision + 1,"changes":expected}));
    let edited = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(s.inspect()["project"]["footprint_assignments"][0]["component"], start[2]["id"]);
    s.apply("edit.undo", &json!({})).unwrap();
    crate::assert_same_content(&before, &s.snapshot().unwrap());
    assert_eq!(s.inspect()["project"]["footprint_assignments"], json!([]));
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), edited);
}

/// An edit prepared against an older revision is refused before validation, for a dry run too.
#[test]
fn a_stale_revision_is_refused() {
    let mut s = fixture();
    let component = rows(&s)[0]["id"].clone();
    for dry_run in [true, false] {
        let params = json!({"edits":[{"component":component,"value":"1k"}],"dry_run":dry_run,"expected_revision":s.revision() - 1});
        let error = s.execute(SET_FIELDS, &params).unwrap_err();
        assert!(matches!(error, CommandError::StaleRevision { current } if current == s.revision()), "{error:?}");
    }
}

/// The same edit on a reopened copy saves the same bytes; the edited save reopens with zero
/// diff, and repeating an applied edit changes nothing and adds no history.
#[test]
fn the_same_edit_saves_the_same_bytes_and_reopens_unchanged() {
    let mut s = fixture();
    let start = rows(&s);
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let edits = json!([
        {"component":start[1]["id"],"value":"22k","fields":{CUSTOM:{"name":"Tolerance","value":"5%"}}},
        {"component":start[5]["id"],"fields":{"description":{"name":"Description","value":"Dual amplifier"}}},
        {"component":start[3]["id"],"footprint":footprint(json!({"1":"2","2":"1"}))},
    ]);
    run(&mut s, edits.clone(), false).unwrap();
    let first = encode_documents(s.snapshot().unwrap()).unwrap();
    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&saved).unwrap());
    run(&mut reopened, edits.clone(), false).unwrap();
    assert_eq!(encode_documents(reopened.snapshot().unwrap()).unwrap(), first);
    assert_eq!(encode_documents(decode_documents(&first).unwrap()).unwrap(), first);
    let revision = s.revision();
    let snapshot = s.snapshot().unwrap();
    assert_eq!(run(&mut s, edits, false).unwrap(), json!({"revision":revision,"changes":[]}));
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
}
