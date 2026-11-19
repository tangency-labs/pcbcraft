use super::*;
use crate::{
    PLACE_RESISTOR, PLACE_SYMBOL, SET_SYMBOL_FIELDS,
    persistence::{decode_packed, encode_packed},
};

/// Place an original built-in symbol and return its placement ID.
fn place(session: &mut Session, definition: &str, x: i64) -> String {
    session.apply(PLACE_SYMBOL, &json!({"definition":definition,"x_nm":x,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    session.project().unwrap().symbols().last().unwrap().id()
}

/// Make a manual displayed designator without changing its internal reference number.
fn reference(session: &mut Session, symbol: &str, reference: &str, locked: bool) {
    session
        .apply(
            SET_SYMBOL_FIELDS,
            &json!({"symbol":symbol,"reference":reference,"value":"fixture","designator_locked":locked,"expected_revision":session.revision()}),
        )
        .unwrap();
}

/// Read the exact command-generated archive, including unexposed reference numbers.
fn archive(session: &Session) -> Vec<u8> {
    encode_packed(session.snapshot().unwrap()).unwrap()
}

/// Applying a batch leaves the locked part untouched, preserves native uniqueness even when a
/// manually edited designator occupied the requested internal number, and undoes in one step.
#[test]
fn annotation_apply_commits_once_and_undo_restores_identical_archive_bytes() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let kept = place(&mut session, "resistor", 0);
    let pending = place(&mut session, "resistor", 4_000_000);
    let second = place(&mut session, "capacitor", 8_000_000);
    reference(&mut session, &kept, "R9", true);
    reference(&mut session, &pending, "R?", false);
    reference(&mut session, &second, "C?", false);
    let before = archive(&session);
    let lock = session.project().unwrap().components().next().unwrap().clone();
    let preview = session.execute(PREVIEW, &json!({})).unwrap();
    let rev = session.revision();
    assert_eq!(preview["annotation_preview"]["rows"][1]["proposed"], "R1");
    let params = json!({"token":preview["token"]});
    assert_eq!(session.apply(APPLY, &params).unwrap(), rev + 1);
    let parts: Vec<_> = session.project().unwrap().components().collect();
    assert_eq!(parts[0], &lock);
    assert_eq!((parts[1].reference(), parts[1].reference_number), ("R1", 2));
    assert_eq!((parts[2].reference(), parts[2].reference_number), ("C1", 1));
    let applied = archive(&session);
    assert_ne!(before, applied);
    assert_eq!(encode_packed(decode_packed(&applied).unwrap().project).unwrap(), applied);
    assert!(matches!(session.apply(APPLY, &params), Err(CommandError::Unavailable(message)) if message.contains("unknown or replaced")));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(archive(&session), before);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(archive(&session), applied);
    let next = session.execute(PREVIEW, &json!({})).unwrap();
    let revision = session.revision();
    assert_eq!(session.apply(APPLY, &json!({"token":next["token"]})).unwrap(), revision, "no changes means no undo entry");
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(archive(&session), before);
}

/// A retained review names changed parts; switching token, opening the same bytes again and
/// undoing to identical authored content still never authorize edits prepared for an old revision.
#[test]
fn annotation_apply_refuses_stale_and_unrecognized_tokens_without_editing() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let symbol = place(&mut session, "resistor", 0);
    let part = session.project().unwrap().components().next().unwrap().id();
    reference(&mut session, &symbol, "R?", false);
    let preview = session.execute(PREVIEW, &json!({})).unwrap();
    let old = json!({"token":preview["token"]});
    reference(&mut session, &symbol, "R4", true);
    let saved = archive(&session);
    let rev = session.revision();
    let stale = session.apply(APPLY, &old).unwrap_err();
    assert_eq!(stale.code(), crate::error_codes::ErrorCode::StaleRevision);
    assert_eq!(stale.details(), json!({"current_revision":rev,"part_instances":[part]}));
    assert!(stale.to_string().contains(&part));
    assert_eq!(archive(&session), saved);
    assert_eq!(session.revision(), rev);
    session.apply("edit.undo", &Value::Null).unwrap();
    let stale = session.apply(APPLY, &old).unwrap_err();
    assert_eq!(stale.details()["part_instances"], json!([]));
    let latest = session.execute(PREVIEW, &json!({})).unwrap();
    assert!(matches!(session.apply(APPLY, &old), Err(CommandError::Unavailable(message)) if message.contains("unknown or replaced")));
    assert!(
        matches!(session.apply(APPLY, &json!({"token":Uuid::nil()})), Err(CommandError::InvalidParams(message)) if message.contains("invalid annotation"))
    );
    session.open_snapshot(session.snapshot().unwrap());
    let before = session.inspect();
    assert!(
        matches!(session.apply(APPLY, &json!({"token":latest["token"]})), Err(CommandError::Unavailable(message)) if message.contains("unknown or replaced"))
    );
    assert_eq!(session.inspect(), before);
}

/// Validate a cycle as one final set of names. A conflicting final name or a locked change is
/// refused before publication; the batch helper never modifies the input snapshot.
#[test]
fn annotation_apply_checks_the_final_designators_not_intermediate_names() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    for x in [0, 4_000_000, 8_000_000] {
        session.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    }
    let project = session.project().unwrap();
    let ids: Vec<_> = project.components.keys().copied().collect();
    let edits = BTreeMap::from([(ids[0], "R2"), (ids[1], "R3"), (ids[2], "R1")]);
    let rotated = with_designators(project, &edits).unwrap();
    assert_eq!(rotated.components().map(|part| (part.reference(), part.reference_number)).collect::<Vec<_>>(), [("R2", 2), ("R3", 3), ("R1", 1)]);
    assert!(
        matches!(with_designators(project, &BTreeMap::from([(ids[0], "R2")])), Err(CommandError::InvalidParams(message)) if message.contains("duplicate reference"))
    );
    let locked = project.symbols()[0].id();
    reference(&mut session, &locked, "R1", true);
    assert!(matches!(with_designators(session.project().unwrap(), &edits), Err(CommandError::InvalidParams(message)) if message.contains("locked")));
}
