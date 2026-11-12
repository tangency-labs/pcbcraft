//! Allocation metadata survives history and persistence without changing authored identity.
use super::*;
use persistence::{Documents, decode, decode_documents, decode_packed, encode, encode_documents, encode_packed};

/// Begin an ordinary new project through the same command used by GUI and MCP.
fn session() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session
}

/// Execute a revision-checked command with no hand-edited positive fixture records.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Place a resistor away from other geometry and return its stable numeric identity.
fn place(session: &mut Session, x_nm: i64) -> u64 {
    apply(session, PLACE_RESISTOR, json!({"x_nm":x_nm,"y_nm":0}));
    session.project().unwrap().symbols().last().unwrap().ordinal
}

/// Reopen using each production persistence route, including retained single-document APIs.
fn round_trips(project: Arc<Project>) -> Vec<Arc<Project>> {
    vec![
        decode(&encode(persistence::diagnostic_snapshot(project.clone())).unwrap()).unwrap(),
        decode_documents(&encode_documents(project.clone()).unwrap()).unwrap(),
        decode_packed(&encode_packed(project).unwrap()).unwrap().project,
    ]
}

/// An allocated then removed highest component cannot be reused after a fresh opening.
#[test]
fn deleted_highest_id_remains_reserved_in_all_encodings() {
    let mut s = session();
    place(&mut s, 0);
    let removed = place(&mut s, 25_400_000);
    apply(&mut s, DELETE_SYMBOL, json!({"symbol":format!("symbol-{removed}")}));
    let snapshot = s.snapshot().unwrap();
    assert_eq!(snapshot.numeric_id_high_water, Some(removed));
    for project in round_trips(snapshot) {
        let mut reopened = Session::default();
        reopened.open_snapshot(project);
        assert!(place(&mut reopened, 50_800_000) > removed);
    }
}

/// Undo restores authored state but retains metadata needed by a later session's allocator.
#[test]
fn undo_then_save_and_restart_cannot_reuse_allocated_ids() {
    let mut s = session();
    let empty = s.snapshot().unwrap();
    let undone = place(&mut s, 0);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_same_content(&empty, &s.snapshot().unwrap());
    assert_eq!(s.project().unwrap().numeric_id_high_water, Some(undone));
    for project in round_trips(s.snapshot().unwrap()) {
        let mut reopened = Session::default();
        reopened.open_snapshot(project);
        assert!(place(&mut reopened, 0) > undone);
    }
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().symbols()[0].ordinal, undone);
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(place(&mut s, 0) > undone);
}

/// Restoring older content in the same live lineage cannot lower allocation metadata.
#[test]
fn adopting_earlier_snapshot_retains_the_current_projects_watermark() {
    let mut s = session();
    let earlier = s.snapshot().unwrap();
    let removed = place(&mut s, 0);
    s.open_snapshot(earlier);
    assert_eq!(s.project().unwrap().numeric_id_high_water, Some(removed));
    let mut restarted = Session::default();
    restarted.open_snapshot(decode_documents(&encode_documents(s.snapshot().unwrap()).unwrap()).unwrap());
    assert!(place(&mut restarted, 0) > removed);
}

/// Value, geometry and view operations do not turn revision advances into allocation metadata.
#[test]
fn edits_without_allocations_leave_the_watermark_unchanged() {
    let mut s = session();
    let id = place(&mut s, 0);
    let high_water = s.project().unwrap().numeric_id_high_water;
    for x_nm in [1_270_000, 2_540_000] {
        apply(&mut s, MOVE_SYMBOL, json!({"symbol":format!("symbol-{id}"),"x_nm":x_nm,"y_nm":0}));
        assert_eq!(s.project().unwrap().numeric_id_high_water, high_water);
    }
    apply(&mut s, SET_SYMBOL_FIELDS, json!({"symbol":format!("symbol-{id}"),"reference":"R5","value":"20k"}));
    assert_eq!(s.project().unwrap().numeric_id_high_water, high_water);
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(reopened).unwrap(), documents);
}

/// Legacy documents keep their canonical old encoding until allocation or retirement requires adoption.
#[test]
fn legacy_noop_and_nonallocating_edits_preserve_old_format() {
    let legacy = decode(include_bytes!("../../app/tests/fixtures/two-resistors.pc")).unwrap();
    assert_eq!(legacy.numeric_id_high_water, None);
    let before = encode_documents(legacy.clone()).unwrap();
    assert_eq!(encode_documents(decode_documents(&before).unwrap()).unwrap(), before);
    let mut s = Session::default();
    s.open_snapshot(legacy);
    apply(&mut s, MOVE_SYMBOL, json!({"symbol":"symbol-2","x_nm":1_270_000,"y_nm":0}));
    assert_eq!(s.project().unwrap().numeric_id_high_water, None);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap()["project.json"], before["project.json"]);
    let highest = s.project().unwrap().highest_id();
    apply(&mut s, DELETE_WIRE, json!({"wire":format!("wire-{highest}")}));
    assert_eq!(s.project().unwrap().numeric_id_high_water, Some(highest));
}

/// Schema failures cannot smuggle allocation metadata into an older version or understate live IDs.
#[test]
fn watermark_formats_are_strict_and_validate_every_retained_identity() {
    let mut s = session();
    let id = place(&mut s, 0);
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    for value in [Value::Null, json!(-1), json!(MAX_OBJECT_ID + 1), json!(id - 1), json!("12")] {
        let mut invalid = documents.clone();
        edit_entry(&mut invalid, |entry| entry["numeric_id_high_water"] = value);
        assert!(decode_documents(&invalid).is_err());
    }
    let mut invalid = documents.clone();
    edit_entry(&mut invalid, |entry| entry["format"] = json!("pcbcraft-project/6"));
    assert!(decode_documents(&invalid).is_err());
    edit_entry(&mut invalid, |entry| {
        entry["format"] = json!("pcbcraft-project/7");
        entry.as_object_mut().unwrap().remove("numeric_id_high_water");
    });
    assert!(decode_documents(&invalid).is_err());
    let mut single: Value = serde_json::from_slice(&encode(persistence::diagnostic_snapshot(s.snapshot().unwrap())).unwrap()).unwrap();
    single["format"] = json!("pcbcraft-project/4");
    assert!(decode(&serde_json::to_vec(&single).unwrap()).is_err());
}

/// Exhaustion is checked by every allocator, including labels and compound wire transactions.
#[test]
fn exhausted_watermark_refuses_allocation_without_partial_mutation() {
    let mut project = session().project().unwrap().clone();
    project.numeric_id_high_water = Some(MAX_OBJECT_ID);
    for (command, params) in [
        (PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0})),
        (net_labels::ADD, json!({"name":"GND","style":"global","x_nm":0,"y_nm":0})),
        (hierarchy::CREATE_CHILD, json!({"name":"Child","x_nm":0,"y_nm":0})),
        (hierarchy::ADD_LABEL, json!({"name":"PORT","direction":"input","x_nm":0,"y_nm":0})),
        (DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":1_270_000,"y_nm":0},"bends":[]})),
    ] {
        let mut s = Session::default();
        s.open_snapshot(Arc::new(project.clone()));
        let before = s.snapshot().unwrap();
        let revision = s.revision();
        let mut params = params;
        params["expected_revision"] = json!(revision);
        let error = s.apply(command, &params).unwrap_err();
        assert!(error.to_string().contains("exhausted"), "{command}: {error}");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()), "{command}");
        assert_eq!(s.revision(), revision);
    }
}

/// An exhausted allocator must not prevent deleting a whole one-leg wire without new IDs.
#[test]
fn exhausted_watermark_still_allows_nonallocating_wire_removal() {
    let mut s = session();
    apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":1_270_000,"y_nm":0},"bends":[]}));
    let wire = s.project().unwrap().wires()[0].ordinal;
    let mut project = s.project().unwrap().clone();
    project.numeric_id_high_water = Some(MAX_OBJECT_ID);
    s.open_snapshot(Arc::new(project));
    apply(&mut s, DELETE_WIRE_SEGMENT, json!({"wire":format!("wire-{wire}"),"at":{"x_nm":635_000,"y_nm":0}}));
    assert!(s.project().unwrap().wires().is_empty());
    assert_eq!(s.project().unwrap().numeric_id_high_water, Some(MAX_OBJECT_ID));
}

/// Change only negative test input; positive fixtures are always command-built and encoded.
fn edit_entry(documents: &mut Documents, edit: impl FnOnce(&mut Value)) {
    let mut entry: Value = serde_json::from_slice(&documents["project.json"]).unwrap();
    edit(&mut entry);
    documents.insert("project.json".into(), serde_json::to_vec(&entry).unwrap());
}
