use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::*;
use crate::{PLACE_RESISTOR, PLACE_SYMBOL, SET_SYMBOL_FIELDS, persistence::encode_packed, sheets};

/// Build an original schematic through the same commands used by the editor.
fn place(session: &mut Session, command: &str, mut params: Value) -> String {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
    session.project().unwrap().all_symbols().max_by_key(|unit| unit.ordinal).unwrap().id()
}

/// Add a flat schematic sheet through the shared command and return its target.
fn second_sheet(session: &mut Session) -> SheetTarget {
    session.apply(sheets::CREATE_SHEET, &json!({"name":"Second","expected_revision":session.revision()})).unwrap();
    session.project().unwrap().sheets().nth(1).unwrap().target
}

/// Set the displayed reference and lock independently of the placement's stored number.
fn reference(session: &mut Session, symbol: &str, reference: &str, locked: bool, sheet: Uuid) {
    session
        .apply(
            SET_SYMBOL_FIELDS,
            &json!({"sheet":sheet,"symbol":symbol,"reference":reference,"designator_locked":locked,"value":"fixture","expected_revision":session.revision()}),
        )
        .unwrap();
}

/// Scoped gaps respect the visible designators of the whole project, and preview preserves
/// identity, archive bytes and both directions of history on a saved/reopened design.
#[test]
fn annotation_preview_fills_gaps_without_editing() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let first = session.project().unwrap().sheet_target().id();
    let reserved = place(&mut session, PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    let second = second_sheet(&mut session);
    let pending = place(&mut session, PLACE_RESISTOR, json!({"sheet":second.id(),"x_nm":2_000_000,"y_nm":2_000_000}));
    let kept = place(&mut session, PLACE_RESISTOR, json!({"sheet":second.id(),"x_nm":6_000_000,"y_nm":-4_000_000}));
    let locked = place(&mut session, PLACE_SYMBOL, json!({"sheet":second.id(),"definition":"capacitor","x_nm":0,"y_nm":0}));
    reference(&mut session, &pending, "R?", false, second.id());
    reference(&mut session, &kept, "R2", false, second.id());
    reference(&mut session, &locked, "C?", true, second.id());
    let saved = encode_packed(session.snapshot().unwrap()).unwrap();
    session.open_snapshot(crate::persistence::decode_packed(&saved).unwrap().project);
    let before_rename = session.snapshot().unwrap();
    session.apply("project.rename", &json!({"name":"Renamed"})).unwrap();
    let renamed = session.snapshot().unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    let baseline = session.snapshot().unwrap();
    let before = session.inspect();
    let revision = session.revision();
    let occurrence = baseline.sheet_occurrence(second).unwrap().id;
    let part_instance = baseline.all_symbols().find(|unit| unit.id() == pending).unwrap().component_id();
    let keeper = baseline.all_symbols().find(|unit| unit.id() == kept).unwrap();
    assert_eq!(baseline.component(keeper).unwrap().reference_number, 3);
    let scoped = session.execute(PREVIEW, &json!({"sheet":second.id(),"expected_revision":revision})).unwrap();
    assert_eq!(scoped["revision"], revision);
    let preview = &scoped["annotation_preview"];
    assert_eq!(preview["errors"], json!([]));
    let rows = preview["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["symbol"], kept);
    assert_eq!(rows[0]["current"], "R2");
    assert_eq!(rows[0]["proposed"], "R2");
    assert_eq!(rows[1]["symbol"], locked);
    assert_eq!(rows[1]["locked"], true);
    assert_eq!(rows[1]["proposed"], "C?");
    // The keeper's displayed R2 reserves the gap, not its internal placement number 3.
    assert_eq!(
        rows[2],
        json!({"part_instance":part_instance,"symbol":pending,"sheet":second.id(),"occurrence":occurrence,"at":{"x_nm":2_000_000,"y_nm":2_000_000},"locked":false,"current":"R?","proposed":"R3"})
    );
    let all = session.execute(PREVIEW, &json!({})).unwrap();
    assert_eq!(all["annotation_preview"]["rows"].as_array().unwrap().len(), 4);
    assert_eq!(all["annotation_preview"]["rows"][0]["symbol"], reserved);
    assert_eq!(all["annotation_preview"]["rows"][0]["sheet"], json!(first));
    assert_eq!(all["annotation_preview"]["rows"][0]["proposed"], "R1");
    assert_eq!(session.apply(PREVIEW, &json!({"sheet":second.id()})).unwrap(), revision);
    assert_eq!(session.inspect(), before);
    assert!(Arc::ptr_eq(&baseline, &session.snapshot().unwrap()));
    assert_eq!(Sha256::digest(encode_packed(session.snapshot().unwrap()).unwrap()), Sha256::digest(saved));
    session.apply("edit.redo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&renamed, &session.snapshot().unwrap()));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before_rename, &session.snapshot().unwrap()));
}

/// Native projects refuse duplicate references. Exercise the planner defensively by passing
/// command-created parts with one duplicated reference, without constructing an invalid file.
#[test]
fn annotation_preview_reports_duplicates_across_scope_without_repairing_them() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    place(&mut session, PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    let second = second_sheet(&mut session);
    place(&mut session, PLACE_RESISTOR, json!({"sheet":second.id(),"x_nm":0,"y_nm":0}));
    let project = session.project().unwrap();
    let first = project.components().next().unwrap();
    let mut duplicate = project.components().nth(1).unwrap().clone();
    duplicate.reference = first.reference().to_owned();
    let mut targets = targets(project, Some(second));
    targets[0].part = &duplicate;
    let report = serde_json::to_value(plan(&targets, [first, &duplicate].into_iter())).unwrap();
    assert_eq!(report["errors"], json!([{"code":"duplicate_reference","reference":"R1","part_instances":[first.id(),duplicate.id()]}]));
    assert_eq!(report["rows"][0]["current"], "R1");
    assert_eq!(report["rows"][0]["proposed"], "R1");
}

/// A cross-sheet part has one designator; narrow scope reports the unit on the selected sheet.
#[test]
fn annotation_preview_uses_one_representative_for_a_multi_unit_part() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session
        .execute(
            "library.load_source",
            &json!({"source":"preview-fixture","documents":{
                "library.json":include_str!("../../../../examples/native-parts/library.json"),
                "symbols/dual-amplifier.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json"),
            }}),
        )
        .unwrap();
    let definition = session.libraries.symbol("parts-demo", "dual-amplifier", 1).unwrap();
    let digest = crate::library::sources::symbol_digest(&definition).unwrap();
    let first = place(
        &mut session,
        PLACE_SYMBOL,
        json!({"library":"parts-demo","definition":"dual-amplifier","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    let part = session.project().unwrap().components().next().unwrap().id();
    let sheet = second_sheet(&mut session).id();
    let second = place(&mut session, crate::PLACE_UNIT, json!({"sheet":sheet,"component":part,"unit":"b","x_nm":0,"y_nm":0}));
    for (params, symbol) in [(json!({}), first), (json!({"sheet":sheet}), second)] {
        let report = session.execute(PREVIEW, &params).unwrap();
        let rows = report["annotation_preview"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["symbol"], symbol);
        assert_eq!(rows[0]["part_instance"], part);
        assert_eq!(report["annotation_preview"]["errors"], json!([]));
    }
}

/// Missing projects and unknown/malformed sheets fail without changing state. An expected
/// revision that no longer matches is refused before the rest of the request is checked.
#[test]
fn annotation_preview_refuses_invalid_scope_or_revision() {
    let mut session = Session::default();
    assert!(matches!(session.execute(PREVIEW, &json!({})), Err(CommandError::Unavailable(_))));
    session.apply("project.new", &json!({})).unwrap();
    let before = session.inspect();
    assert_eq!(session.execute(PREVIEW, &json!({})).unwrap()["annotation_preview"]["rows"], json!([]));
    assert!(matches!(session.execute(PREVIEW, &json!({"sheet":"wrong"})), Err(CommandError::InvalidParams(_))));
    assert!(matches!(session.execute(PREVIEW, &json!({"sheet":Uuid::nil()})), Err(CommandError::NotFound(_))));
    assert!(matches!(session.execute(PREVIEW, &json!({"expected_revision":0,"sheet":"wrong"})), Err(CommandError::StaleRevision { .. })));
    assert_eq!(session.inspect(), before);
}

/// Counts the shared command's work without a wall-clock requirement or non-test overhead.
#[derive(Clone, Copy, Default)]
pub(super) struct Work {
    pub(super) occurrences: usize,
    pub(super) sort_keys: usize,
    pub(super) units: usize,
    pub(super) probes: usize,
    pub(super) references: usize,
    pub(super) rows: usize,
}

thread_local! {
    static WORK: std::cell::Cell<Option<Work>> = const { std::cell::Cell::new(None) };
}

/// Record work only while this thread is running the large-design regression.
pub(super) fn count_work(update: impl FnOnce(&mut Work)) {
    WORK.with(|counter| {
        if let Some(mut work) = counter.get() {
            update(&mut work);
            counter.set(Some(work));
        }
    });
}

/// Exercise the same command as the live editor, but count operations rather than fixture time.
/// Even a one-row review indexes all 2,000 references once; it must not sort off-sheet units.
#[test]
fn annotation_preview_2000_parts_bounds_shared_command_work() {
    const PARTS: usize = 2_000;
    const SPACING: i64 = crate::schematic::GRID_NM * 6;
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let first = session.project().unwrap().sheet_target().id();
    let sheet = second_sheet(&mut session);
    for i in 0..PARTS - 1 {
        let grid = (i * 997) % PARTS;
        place(&mut session, PLACE_RESISTOR, json!({"sheet":first,"x_nm":(grid % 50) as i64 * SPACING,"y_nm":(grid / 50) as i64 * SPACING}));
    }
    let placeholder = place(&mut session, PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":0,"y_nm":0}));
    reference(&mut session, &placeholder, "R?", false, sheet.id());
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    for (params, count) in [(json!({"sheet":sheet.id()}), 1), (json!({}), PARTS)] {
        WORK.set(Some(Work::default()));
        let result = session.execute(PREVIEW, &params);
        let work = WORK.take().unwrap();
        let result = result.unwrap();
        let rows = result["annotation_preview"]["rows"].as_array().unwrap();
        assert_eq!(result["annotation_preview"]["errors"], json!([]));
        assert_eq!(rows.len(), count);
        let pending = rows.iter().find(|row| row["symbol"] == placeholder).unwrap();
        assert_eq!(pending["proposed"], "R2000");
        assert_eq!((work.occurrences, work.references, work.units, work.rows), (2, PARTS, count, count));
        assert_eq!(work.probes, PARTS, "one gap search across the occupied prefix");
        if count == 1 {
            assert!(work.sort_keys <= 2, "do not sort off-sheet units for a scoped preview");
        } else {
            assert!(work.sort_keys <= 4 * PARTS * PARTS.ilog2() as usize, "sort key evaluations: {}", work.sort_keys);
        }
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
}

/// The last allowed reference number may be proposed; the following one may not.
#[test]
fn annotation_preview_stops_at_the_reference_number_limit() {
    let occupied = BTreeMap::new();
    assert_eq!(first_free_reference("R", MAX_REFERENCE_NUMBER, &occupied), Some(format!("R{MAX_REFERENCE_NUMBER}")));
    assert_eq!(first_free_reference("R", MAX_REFERENCE_NUMBER + 1, &occupied), None);
}
