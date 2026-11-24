use super::*;
use crate::{
    PLACE_SYMBOL, Session,
    authoring::{Item, Kind as DraftKind, publishing},
    library::sources::{Catalog, Documents, Kind, LOAD, NativeSource, symbol_digest},
    persistence::{decode_documents, encode_documents},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Build the target through the native authoring encoder and assertions through their own codec.
fn source() -> (Documents, Assertion, Assertion) {
    source_for(Arc::new(Item::new(DraftKind::Symbol).unwrap()))
}

/// Publish one authored symbol draft and two assertions about it.
fn source_for(item: Arc<Item>) -> (Documents, Assertion, Assertion) {
    let manifest = publishing::new_library("lifecycle-demo", "Lifecycle demo", "MIT OR Apache-2.0").unwrap();
    let native = publishing::prepare(item, &manifest, 0, None, false).unwrap();
    let symbol = native.item().symbol().unwrap();
    let draft = Assertion {
        format: FORMAT.into(),
        address: Reference { library: symbol.library.clone(), id: "drawing-assessment".into(), revision: 1 },
        target: Key { library: symbol.library.clone(), kind: Kind::Symbol, id: symbol.id.clone(), revision: symbol.revision() },
        scheme: Policy::standard().scheme,
        state: "draft".into(),
        reason: "Pending drawing review".into(),
        provenance: Provenance { publisher: "Lifecycle demo".into() },
    };
    let mut released = draft.clone();
    released.address.revision = 2;
    released.state = "released".into();
    released.reason = "Drawing review completed".into();
    let mut documents = native.documents().clone();
    documents.insert("library.json".into(), manifest);
    documents.insert("lifecycle-assertions/drawing-assessment.r1.json".into(), draft.encode().unwrap());
    documents.insert("lifecycle-assertions/drawing-assessment.r2.json".into(), released.encode().unwrap());
    (documents, draft, released)
}

/// Address the exact assertion and its selection digest with the current live project revision.
fn params(session: &Session, assertion: &Assertion) -> Value {
    let address = &assertion.address;
    json!({"library":address.library,"assertion":address.id,"assertion_revision":address.revision,
        "expected_revision":session.revision(),"digest":assertion.digest().unwrap()})
}

/// Load a native source into a synthetic project, then place one exact revision through the shared command.
fn placed() -> (Session, Documents, Assertion, Assertion) {
    let (documents, draft, released) = source();
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
    let key = &draft.target;
    let symbol = session.libraries().item(key).unwrap().symbol().unwrap();
    session
        .apply(
            PLACE_SYMBOL,
            &json!({"library":key.library,"definition":key.id,"definition_revision":key.revision,
        "definition_digest":symbol_digest(&symbol).unwrap(),"expected_revision":session.revision(),"x_nm":0,"y_nm":0}),
        )
        .unwrap();
    (session, documents, draft, released)
}

/// A preview does not mutate; each adoption is one undo step and the offline round trip preserves exact state and bytes.
#[test]
fn assessment_round_trips_offline_and_adopts_in_one_undo_step() {
    let (mut session, _, draft, released) = placed();
    let starting = encode_documents(session.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&starting["project.json"]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/16");
    assert!(entry.get("board_instances").is_none());
    assert!(!starting.contains_key("library/lifecycle.json"));
    let pre_revision = session.revision();
    let mut preview = params(&session, &draft);
    preview["dry_run"] = json!(true);
    let result = session.execute(ADOPT, &preview).unwrap();
    assert_eq!(result["disposition_after"], "draft");
    assert_eq!(session.revision(), pre_revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), starting);
    session.execute(ADOPT, &params(&session, &draft)).unwrap();
    let draft_files = encode_documents(session.snapshot().unwrap()).unwrap();
    let draft_entry: Value = serde_json::from_slice(&draft_files["project.json"]).unwrap();
    assert_eq!(draft_entry["format"], "pcbcraft-project/64");
    assert!(draft_files.contains_key("library/lifecycle.json"));
    let changed = session.execute(ADOPT, &params(&session, &released)).unwrap();
    assert_eq!(changed["before"]["revision"], 1);
    assert_eq!(changed["after"]["revision"], 2);
    assert_eq!(changed["disposition_after"], "released");
    assert_eq!(session.revision(), pre_revision + 2);
    assert_eq!(session.execute(ADOPT, &params(&session, &released)).unwrap()["changed"], false);
    let released_files = encode_documents(session.snapshot().unwrap()).unwrap();
    for (path, bytes) in &starting {
        if path != crate::persistence::ENTRY_POINT {
            assert_eq!(&released_files[path], bytes, "unrelated document {path}");
        }
    }
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), draft_files);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), starting);
    session.apply("edit.redo", &Value::Null).unwrap();
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), released_files);
    let mut offline = Session::default();
    offline.open_snapshot(decode_documents(&released_files).unwrap());
    assert!(offline.libraries().lifecycle_assertion(&released.address).unwrap_err().contains("unavailable"));
    assert_eq!(offline.execute(INSPECT, &json!({})).unwrap()["assessments"][0]["disposition"], "released");
    assert_eq!(encode_documents(offline.snapshot().unwrap()).unwrap(), released_files);
    let packed = crate::persistence::encode_packed_documents(released_files.clone()).unwrap();
    assert_eq!(crate::persistence::decode_packed(&packed).unwrap().documents, released_files);
}

/// Snapshot the revision, history root and undo/redo flags to assert refused edits touch none of them.
fn history_state(session: &Session) -> (u64, u64, bool, bool) {
    let inspected = session.inspect();
    (session.revision(), session.history_root_revision(), inspected["can_undo"].as_bool().unwrap(), inspected["can_redo"].as_bool().unwrap())
}

/// Publisher refresh can expose more information but may not change a pin or overwrite immutable assertion bytes.
#[test]
fn conflict_and_stale_selection_do_not_replace_a_pinned_assertion() {
    let (mut session, mut documents, draft, mut released) = placed();
    let pre_adopt = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut stale = params(&session, &released);
    session.apply(ADOPT, &params(&session, &draft)).unwrap();
    assert_eq!(session.execute(LOOKUP, &params_for_lookup(&released)).unwrap()["digest"], released.digest().unwrap());
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let before_history = history_state(&session);
    assert!(session.execute(ADOPT, &stale).unwrap_err().to_string().contains("project changed"));
    assert_eq!(history_state(&session), before_history);
    stale["expected_revision"] = json!(session.revision());
    stale["digest"] = json!("0".repeat(64));
    assert!(session.execute(ADOPT, &stale).unwrap_err().to_string().contains("content changed"));
    assert_eq!(history_state(&session), before_history);
    released.state = "obsolete".into();
    documents.insert("lifecycle-assertions/drawing-assessment.r2.json".into(), released.encode().unwrap());
    session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
    assert!(session.execute(LOOKUP, &params_for_lookup(&released)).unwrap_err().to_string().contains("Conflicting"));
    assert_eq!(history_state(&session), before_history);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    assert_eq!(session.snapshot().unwrap().lifecycle().disposition(&draft.target), Some(Disposition::Draft));
    // Undo steps back to pre-adoption in one step, proving no refused edit entered history.
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), pre_adopt);
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// Build the three exact address fields that the read-only source command accepts.
fn params_for_lookup(assertion: &Assertion) -> Value {
    json!({"library":assertion.address.library,"assertion":assertion.address.id,"assertion_revision":assertion.address.revision})
}

/// A read-only session runs both lifecycle reads through each dispatch path, while adoption is
/// refused before validation. The project, revision and history stay exactly as opened.
#[test]
fn read_only_sessions_read_lifecycle_state_without_changing_project_or_history() {
    use crate::{SessionScope, error_codes::ErrorCode};
    let (mut editor, documents, draft, released) = placed();
    editor.apply(ADOPT, &params(&editor, &draft)).unwrap();
    let project = editor.snapshot().unwrap();
    let saved = encode_documents(project.clone()).unwrap();
    let mut reader = Session::with_scope(SessionScope::ReadOnly);
    reader.open_snapshot(project.clone());
    reader.install_library_source("fixture", Arc::new(crate::library::sources::NativeSource::load(&documents).unwrap())).unwrap();
    let before = reader.inspect();
    let (revision, root) = (reader.revision(), reader.history_root_revision());
    let lookup = reader.execute(LOOKUP, &params_for_lookup(&released)).unwrap();
    assert_eq!(lookup, json!({"assertion":released,"digest":released.digest().unwrap()}));
    let inspected = reader.execute(INSPECT, &json!({})).unwrap();
    assert_eq!(inspected["revision"], revision);
    assert_eq!(inspected["assessments"], json!([{"assertion":draft,"disposition":"draft"}]));
    assert_eq!(reader.apply(LOOKUP, &params_for_lookup(&released)).unwrap(), revision);
    assert_eq!(reader.apply(INSPECT, &json!({})).unwrap(), revision);
    for request in [params(&reader, &released), json!({"unexpected":true})] {
        for refused in [reader.execute(ADOPT, &request).unwrap_err(), reader.apply(ADOPT, &request).unwrap_err()] {
            assert_eq!(refused.code(), ErrorCode::OutOfScope);
            assert_eq!(refused.details(), json!({"command":ADOPT,"scope":"read_only"}));
        }
    }
    assert_eq!(reader.inspect(), before);
    assert_eq!((reader.revision(), reader.history_root_revision()), (revision, root));
    assert!(Arc::ptr_eq(&project, &reader.snapshot().unwrap()));
    assert_eq!(encode_documents(reader.snapshot().unwrap()).unwrap(), saved);
    assert_eq!(reader.snapshot().unwrap().lifecycle().disposition(&draft.target), Some(Disposition::Draft));
}

/// An unknown scheme remains unknown, and a supplied policy changes classification only for its exact scheme revision.
#[test]
fn custom_and_unknown_schemes_are_not_evaluated_by_label() {
    let (mut session, mut documents, mut draft, _) = placed();
    draft.address.revision = 3;
    draft.scheme.library = "lifecycle-demo".into();
    draft.scheme.revision = 2;
    documents.insert("lifecycle-assertions/custom.json".into(), draft.encode().unwrap());
    session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
    assert!(session.execute(ADOPT, &params(&session, &draft)).unwrap()["disposition_after"].is_null());
    let mut edit = params(&session, &draft);
    let mut policy = Policy {
        scheme: draft.scheme.clone(),
        states: vec![State { id: "draft".into(), label: "Production".into(), disposition: Disposition::Deprecated }],
    };
    let before_refusal = encode_documents(session.snapshot().unwrap()).unwrap();
    let before_history = history_state(&session);
    policy.scheme.revision = 1;
    edit["policy"] = json!(policy);
    assert!(session.execute(ADOPT, &edit).unwrap_err().to_string().contains("exact scheme revision"));
    assert_eq!(history_state(&session), before_history);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_refusal);
    policy.scheme.revision = 2;
    edit["policy"] = json!(policy);
    assert_eq!(session.execute(ADOPT, &edit).unwrap()["disposition_after"], "deprecated");
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before_refusal);
    assert_eq!(session.snapshot().unwrap().lifecycle().disposition(&draft.target), None);
}

/// Refuse invalid publisher documents and policy duplicates instead of interpreting malformed metadata.
#[test]
fn codecs_refuse_wrong_versions_unknown_fields_and_limits() {
    let (_, mut draft, _) = source();
    draft.reason = "é".repeat(MAX_NOTE_CHARS);
    draft.provenance.publisher = "é".repeat(MAX_LABEL_CHARS);
    let encoded = draft.encode().unwrap();
    assert_eq!(Assertion::read(&encoded).unwrap(), draft);
    for bad_reason in [
        "",
        " ",
        " Drawing review",
        "Drawing review ",
        "\u{feff}A",
        "A\u{feff}",
        "Drawing\nreview",
        "Drawing\treview",
        "A\0B",
        &"é".repeat(MAX_NOTE_CHARS + 1),
    ] {
        let mut invalid = draft.clone();
        invalid.reason = bad_reason.into();
        assert!(invalid.encode().is_err(), "reason {bad_reason:?}");
    }
    for bad_publisher in ["", " ", " Publisher", "Publisher ", "\u{feff}A", "A\u{feff}", "Pub\nlisher", "A\x7fB", &"é".repeat(MAX_LABEL_CHARS + 1)] {
        let mut invalid = draft.clone();
        invalid.provenance.publisher = bad_publisher.into();
        assert!(invalid.encode().is_err(), "publisher {bad_publisher:?}");
    }
    let mut value: Value = serde_json::from_str(&encoded).unwrap();
    value["target"]["unexpected"] = json!(true);
    assert!(Assertion::read(&value.to_string()).unwrap_err().contains("unknown field `unexpected`"));
    value = serde_json::from_str(&encoded).unwrap();
    value["format"] = json!("pcbcraft-lifecycle-assertion/2");
    assert!(Assertion::read(&value.to_string()).unwrap_err().contains("Unsupported lifecycle assertion format"));
    let mut policy = Policy::standard();
    policy.states =
        (0..MAX_STATES).map(|i| State { id: format!("state-{i}"), label: "é".repeat(MAX_LABEL_CHARS), disposition: Disposition::Draft }).collect();
    assert!(policy.validate().is_ok());
    for bad_label in ["", " ", " Released", "Released ", "\u{feff}A", "A\u{feff}", "Re\nleased", "A\x7fB", &"é".repeat(MAX_LABEL_CHARS + 1)] {
        let mut invalid = Policy::standard();
        invalid.states[0].label = bad_label.into();
        assert!(invalid.validate().is_err(), "label {bad_label:?}");
    }
    policy.states.push(State { id: "extra".into(), label: "Extra".into(), disposition: Disposition::Draft });
    assert!(policy.validate().unwrap_err().contains("must contain 1 to 64 states"));
    policy.states.truncate(2);
    policy.states[1].id = policy.states[0].id.clone();
    assert!(policy.validate().unwrap_err().contains("Duplicate lifecycle state ID"));
}

/// A lifecycle document and the project/64 header must appear together and stay valid.
#[test]
fn lifecycle_file_and_project_header_must_agree() {
    let (mut session, _, draft, _) = placed();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&before["project.json"]).unwrap()["format"], "pcbcraft-project/16");
    session.apply(ADOPT, &params(&session, &draft)).unwrap();
    let after = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&after["project.json"]).unwrap()["format"], "pcbcraft-project/64");
    let mut missing = after.clone();
    missing.remove("library/lifecycle.json");
    assert!(decode_documents(&missing).unwrap_err().contains("project/64 requires library/lifecycle.json"));
    let orphan = edit_project_header(&after, |header| header["format"] = json!("pcbcraft-project/16"));
    assert!(decode_documents(&orphan).unwrap_err().contains("requires project/64"));
    let mut empty = after.clone();
    empty.insert("library/lifecycle.json".into(), br#"{"assertions":[],"policies":[]}"#.to_vec());
    assert!(decode_documents(&empty).unwrap_err().contains("assessment or policy count"));
    let overlay = edit_project_header(&after, |header| header["content_format"] = json!("pcbcraft-project/16"));
    assert!(decode_documents(&overlay).unwrap_err().contains("unknown field `content_format`"));
    for unshipped in ["pcbcraft-project/47", "pcbcraft-project/52", "pcbcraft-project/58"] {
        let header = edit_project_header(&before, |header| header["format"] = json!(unshipped));
        assert!(decode_documents(&header).unwrap_err().contains(&format!("unsupported project format {unshipped}")));
    }
    // Clearing the last adopted assessment from a reopened /64 project reverts to the prior format.
    let mut cleared = (*decode_documents(&after).unwrap()).clone();
    cleared.lifecycle = Arc::default();
    assert_eq!(encode_documents(Arc::new(cleared)).unwrap(), before);
}

/// Edit only the project entry point, keeping the exact other documents.
fn edit_project_header(documents: &crate::persistence::Documents, edit: impl FnOnce(&mut Value)) -> crate::persistence::Documents {
    use crate::persistence::ENTRY_POINT;
    let mut project: Value = serde_json::from_slice(&documents[ENTRY_POINT]).unwrap();
    edit(&mut project);
    let mut changed = documents.clone();
    changed.insert(ENTRY_POINT.into(), serde_json::to_vec(&project).unwrap());
    changed
}

/// Encode a project lifecycle document from exact records, bypassing the adopting checks.
fn stored(assertions: &[&Assertion], policies: &[&Policy]) -> Vec<u8> {
    serde_json::to_vec(&json!({"assertions":assertions,"policies":policies})).unwrap()
}

/// A persisted document cannot pin two contents at one address, two assertions for one target,
/// or a duplicated or unused policy; the legacy single-file encoder refuses to drop assessments.
#[test]
fn assessment_documents_refuse_duplicates_orphans_and_lossy_legacy_encoding() {
    let (_, draft, released) = source();
    let standard = Policy::standard();
    assert!(Assessments::decode(&stored(&[&draft], &[&standard])).unwrap().disposition(&draft.target) == Some(Disposition::Draft));
    let mut other_target = draft.clone();
    other_target.target.id = "other-symbol".into();
    let mut custom = standard.clone();
    custom.scheme.library = "lifecycle-demo".into();
    for (bytes, message) in [
        (stored(&[&draft, &other_target], &[]), "Duplicate lifecycle assertion address or target"),
        (stored(&[&draft, &released], &[]), "Duplicate lifecycle assertion address or target"),
        (stored(&[&draft], &[&standard, &standard]), "duplicated or has no pinned assertion"),
        (stored(&[&draft], &[&custom]), "duplicated or has no pinned assertion"),
    ] {
        assert!(Assessments::decode(&bytes).unwrap_err().contains(message), "{message}");
    }
    let mut blank = Session::default();
    blank.apply("project.new", &json!({})).unwrap();
    let mut project = crate::persistence::diagnostic_snapshot(blank.snapshot().unwrap());
    crate::persistence::encode(project.clone()).unwrap();
    Arc::make_mut(&mut project).lifecycle = Arc::new(Assessments::decode(&stored(&[&draft], &[&standard])).unwrap());
    assert!(crate::persistence::encode(project).unwrap_err().contains("Lifecycle assessments require split documents"));
}

/// Source loading diagnoses an assertion published under another library instead of indexing it.
#[test]
fn source_refuses_an_assertion_from_another_library() {
    let (mut documents, mut draft, _) = source();
    draft.address.library = "other-library".into();
    draft.target.library = "other-library".into();
    documents.insert("lifecycle-assertions/foreign.json".into(), draft.encode().unwrap());
    let loaded = crate::library::sources::NativeSource::load(&documents).unwrap();
    assert!(
        loaded
            .problems()
            .iter()
            .any(|problem| problem.starts_with("lifecycle-assertions/foreign.json") && problem.contains("does not match library.json")),
        "{:?}",
        loaded.problems()
    );
}

/// A reopened project keeps its pin; a source with different content at that address cannot replace it.
#[test]
fn reopened_pin_refuses_different_content_at_the_same_address() {
    let (mut session, mut documents, draft, _) = placed();
    session.apply(ADOPT, &params(&session, &draft)).unwrap();
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut changed = draft.clone();
    changed.state = "obsolete".into();
    documents.insert("lifecycle-assertions/drawing-assessment.r1.json".into(), changed.encode().unwrap());
    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&saved).unwrap());
    reopened.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
    let before_history = history_state(&reopened);
    let error = reopened.execute(ADOPT, &params(&reopened, &changed)).unwrap_err().to_string();
    assert!(error.contains("already has different pinned content"), "{error}");
    assert_eq!(history_state(&reopened), before_history);
    assert_eq!(encode_documents(reopened.snapshot().unwrap()).unwrap(), saved);
    assert_eq!(reopened.snapshot().unwrap().lifecycle().disposition(&draft.target), Some(Disposition::Draft));
}

/// Lock the only placed part instance through the shared field command, keeping its visible fields.
fn lock_first(session: &mut Session) {
    let project = session.project().unwrap();
    let symbol = project.symbols()[0].id();
    let part = project.components().next().unwrap();
    let params = json!({"symbol":symbol,"reference":part.reference,"value":part.value,"designator_locked":true,
        "expected_revision":session.revision()});
    session.apply(crate::SET_SYMBOL_FIELDS, &params).unwrap();
}

/// Without adopted assessments, locking selects project/51 as on main; adopting an assessment
/// upgrades to project/64 (with board_instances when board-owned) and removing it returns to /51.
#[test]
fn designator_lock_and_assessment_share_project_64_and_read_historical_51() {
    for board in [false, true] {
        let (documents, draft, _) = source();
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":board})).unwrap();
        session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
        let key = &draft.target;
        let symbol = session.libraries().item(key).unwrap().symbol().unwrap();
        let place = json!({"library":key.library,"definition":key.id,"definition_revision":key.revision,
            "definition_digest":symbol_digest(&symbol).unwrap(),"expected_revision":session.revision(),"x_nm":0,"y_nm":0});
        session.apply(PLACE_SYMBOL, &place).unwrap();
        lock_first(&mut session);
        let locked = encode_documents(session.snapshot().unwrap()).unwrap();
        let entry: Value = serde_json::from_slice(&locked[crate::persistence::ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/51");
        assert!(entry.get("board_instances").is_none());
        assert!(entry.get("layout_root").is_some());
        assert!(!locked.contains_key("library/lifecycle.json"));
        let bad_51_instances = edit_project_header(&locked, |header| header["board_instances"] = json!(true));
        assert!(decode_documents(&bad_51_instances).unwrap_err().contains("board_instances requires project/64"));
        session.apply(ADOPT, &params(&session, &draft)).unwrap();
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        let entry: Value = serde_json::from_slice(&saved[crate::persistence::ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/64");
        assert_eq!(entry.get("board_instances"), board.then_some(&Value::Bool(true)));
        assert_eq!(entry.get("schematic_instances").is_some(), !board);
        assert!(saved.contains_key("library/lifecycle.json"));
        let reopened = decode_documents(&saved).unwrap();
        assert!(reopened.components().next().unwrap().designator_locked());
        assert_eq!(reopened.lifecycle(), session.snapshot().unwrap().lifecycle());
        assert!(!reopened.lifecycle().is_empty());
        assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
        let packed = crate::persistence::encode_packed(reopened).unwrap();
        assert_eq!(crate::persistence::decode_packed(&packed).unwrap().documents, saved);
        for (bad, reason) in [(Value::Null, "expected a boolean"), (json!(false), "invalid board_instances"), (json!("true"), "expected a boolean")] {
            let invalid = edit_project_header(&saved, |header| header["board_instances"] = bad);
            let error = decode_documents(&invalid).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
        if let Some(path) = session.project().unwrap().instances_document_path() {
            let mut missing = saved.clone();
            missing.remove(&path);
            assert_eq!(decode_documents(&missing).unwrap_err(), format!("{path}: missing project document"));
            let undeclared = edit_project_header(&saved, |header| {
                header.as_object_mut().unwrap().remove("board_instances");
            });
            assert!(decode_documents(&undeclared).unwrap_err().contains("require instance records"));
            let undeclared_unlocked = edit_project_header(&undeclared, |header| {
                for component in header["components"].as_array_mut().unwrap() {
                    component.as_object_mut().unwrap().remove("designator_locked");
                }
            });
            assert!(decode_documents(&undeclared_unlocked).unwrap_err().contains("unexpected or unused project document"));
        }

        // Project/51 cannot carry the lifecycle document.
        let lifecycle_51 = edit_project_header(&saved, |header| {
            header["format"] = json!("pcbcraft-project/51");
            header.as_object_mut().unwrap().remove("board_instances");
        });
        assert!(decode_documents(&lifecycle_51).unwrap_err().contains("requires project/64"));

        // Undoing the adoption reverts cleanly to project/51 with exact prior bytes.
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), locked);
    }
}

/// A starter symbol whose only pin offers one alternate function (a version-5 definition).
fn alternate_symbol() -> Arc<Item> {
    use crate::authoring::{Action, Primitive};
    use crate::schematic::{AlternatePinFunction, ElectricalType};
    let item = Item::new(DraftKind::Symbol).unwrap();
    let mut pin = item.objects[&1].clone();
    let Primitive::Pin { name, electrical_type, alternates, .. } = &mut pin else { panic!("expected starter pin") };
    *name = Some("GPIO".into());
    *electrical_type = ElectricalType::Bidirectional;
    *alternates = vec![AlternatePinFunction { name: "ADC_IN".into(), electrical_type: ElectricalType::Input }];
    let mut editor = crate::authoring::Session::new(item, false).unwrap();
    editor.execute(0, Action::Replace { object: 1, primitive: pin }).unwrap();
    editor.snapshot()
}

/// An adopted assessment and a chosen alternate pin function together select project/64;
/// both reopen intact and re-save byte-identically. The unreleased /52 and /58 numbers are refused.
#[test]
fn assessment_with_pin_function_selects_project_64() {
    use crate::persistence::ENTRY_POINT;
    for board in [false, true] {
        let (documents, draft, _) = source_for(alternate_symbol());
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":board})).unwrap();
        session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
        let key = &draft.target;
        let symbol = session.libraries().item(key).unwrap().symbol().unwrap();
        let place = json!({"library":key.library,"definition":key.id,"definition_revision":key.revision,
            "definition_digest":symbol_digest(&symbol).unwrap(),"expected_revision":session.revision(),"x_nm":0,"y_nm":0});
        session.apply(PLACE_SYMBOL, &place).unwrap();
        let target = json!({"part":session.project().unwrap().symbols()[0].component_id(),"terminal":"1"});
        let choice = json!({"target":target,"function":"ADC_IN","expected_revision":session.revision()});
        session.apply(crate::pin_functions::SET, &choice).unwrap();
        let chosen = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&chosen[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/48");
        session.apply(ADOPT, &params(&session, &draft)).unwrap();
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        let entry: Value = serde_json::from_slice(&saved[ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/64");
        assert_eq!(entry["components"][0]["pin_functions"], json!([{"terminal":"1","function":"ADC_IN"}]));
        assert_eq!(entry.get("board_instances"), board.then_some(&Value::Bool(true)));
        assert!(saved.contains_key("library/lifecycle.json"));
        let reopened = decode_documents(&saved).unwrap();
        assert_eq!(reopened.lifecycle(), session.snapshot().unwrap().lifecycle());
        assert_eq!(reopened.lifecycle().disposition(&draft.target), Some(Disposition::Draft));
        assert_eq!(reopened.pin_function(reopened.components().next().unwrap(), "1"), Some("ADC_IN"));
        assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
        let packed = crate::persistence::encode_packed(reopened).unwrap();
        assert_eq!(crate::persistence::decode_packed(&packed).unwrap().documents, saved);
        for retired in ["pcbcraft-project/52", "pcbcraft-project/58"] {
            let header = edit_project_header(&saved, |header| header["format"] = json!(retired));
            let error = decode_documents(&header).unwrap_err();
            assert!(error.contains("library/lifecycle.json requires project/64"), "{error}");
        }
        // Undoing the adoption returns to the exact pin-function-only bytes.
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), chosen);
    }
}

/// A project/56 project (a chosen alternate pin function and an active designator lock) that
/// adopts a lifecycle assessment saves as project/64 with both records and reopens intact.
/// The /56 bytes themselves still reopen and re-save unchanged, and Undo restores them exactly.
#[test]
fn pin_function_and_lock_project_56_with_assessment_saves_as_project_64() {
    use crate::persistence::ENTRY_POINT;
    for board in [false, true] {
        let (documents, draft, _) = source_for(alternate_symbol());
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":board})).unwrap();
        session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
        let key = &draft.target;
        let symbol = session.libraries().item(key).unwrap().symbol().unwrap();
        let place = json!({"library":key.library,"definition":key.id,"definition_revision":key.revision,
            "definition_digest":symbol_digest(&symbol).unwrap(),"expected_revision":session.revision(),"x_nm":0,"y_nm":0});
        session.apply(PLACE_SYMBOL, &place).unwrap();
        let target = json!({"part":session.project().unwrap().symbols()[0].component_id(),"terminal":"1"});
        let choice = json!({"target":target,"function":"ADC_IN","expected_revision":session.revision()});
        session.apply(crate::pin_functions::SET, &choice).unwrap();
        lock_first(&mut session);
        let locked = encode_documents(session.snapshot().unwrap()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&locked[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/56");
        assert!(!locked.contains_key("library/lifecycle.json"));
        assert_eq!(encode_documents(decode_documents(&locked).unwrap()).unwrap(), locked);

        session.apply(ADOPT, &params(&session, &draft)).unwrap();
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        let entry: Value = serde_json::from_slice(&saved[ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/64");
        assert_eq!(entry["components"][0]["pin_functions"], json!([{"terminal":"1","function":"ADC_IN"}]));
        assert_eq!(entry["components"][0]["designator_locked"], json!(true));
        assert_eq!(entry.get("board_instances"), board.then_some(&Value::Bool(true)));
        assert!(saved.contains_key("library/lifecycle.json"));
        let reopened = decode_documents(&saved).unwrap();
        let part = reopened.components().next().unwrap();
        assert!(part.designator_locked());
        assert_eq!(reopened.pin_function(part, "1"), Some("ADC_IN"));
        assert_eq!(reopened.lifecycle(), session.snapshot().unwrap().lifecycle());
        assert_eq!(reopened.lifecycle().disposition(&draft.target), Some(Disposition::Draft));
        assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
        let packed = crate::persistence::encode_packed(reopened).unwrap();
        assert_eq!(crate::persistence::decode_packed(&packed).unwrap().documents, saved);

        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), locked);
    }
}

/// Ordinary edits without adopted assessments stay at the prior format version; adopting an
/// assessment selects /64, and removing the last assessment returns to the prior format.
#[test]
fn ordinary_edit_without_assessments_preserves_prior_format_and_removing_last_assessment_reverts() {
    for board in [false, true] {
        let (documents, draft, _) = source();
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":board})).unwrap();
        session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
        let key = &draft.target;
        let symbol = session.libraries().item(key).unwrap().symbol().unwrap();
        let place = json!({"library":key.library,"definition":key.id,"definition_revision":key.revision,
            "definition_digest":symbol_digest(&symbol).unwrap(),"expected_revision":session.revision(),"x_nm":0,"y_nm":0});
        session.apply(PLACE_SYMBOL, &place).unwrap();
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        let before_entry: Value = serde_json::from_slice(&before[crate::persistence::ENTRY_POINT]).unwrap();
        assert_eq!(before_entry["format"], "pcbcraft-project/16");
        assert!(before_entry.get("board_instances").is_none());
        assert!(!before.contains_key("library/lifecycle.json"));

        // An ordinary edit that rewrites project.json keeps project/16 and round-trips cleanly.
        let symbol_id = session.project().unwrap().symbols()[0].id();
        let edit = json!({"symbol":symbol_id,"reference":"U1","value":"REV-A","expected_revision":session.revision()});
        session.apply(crate::SET_SYMBOL_FIELDS, &edit).unwrap();
        let edited = encode_documents(session.snapshot().unwrap()).unwrap();
        let edited_entry: Value = serde_json::from_slice(&edited[crate::persistence::ENTRY_POINT]).unwrap();
        assert_eq!(edited_entry["format"], "pcbcraft-project/16");
        assert!(edited_entry.get("board_instances").is_none());
        assert!(!edited.contains_key("library/lifecycle.json"));
        assert_eq!(encode_documents(decode_documents(&edited).unwrap()).unwrap(), edited);

        // Adopting an assessment selects project/64 and writes library/lifecycle.json.
        session.apply(ADOPT, &params(&session, &draft)).unwrap();
        let adopted = encode_documents(session.snapshot().unwrap()).unwrap();
        let adopted_entry: Value = serde_json::from_slice(&adopted[crate::persistence::ENTRY_POINT]).unwrap();
        assert_eq!(adopted_entry["format"], "pcbcraft-project/64");
        assert_eq!(adopted_entry.get("board_instances"), board.then_some(&Value::Bool(true)));
        assert!(adopted.contains_key("library/lifecycle.json"));
        for (path, bytes) in &edited {
            if path != crate::persistence::ENTRY_POINT {
                assert_eq!(&adopted[path], bytes, "unrelated document {path}");
            }
        }

        // Removing the last assessment (via Undo or on a reopened /64 snapshot) returns to project/16.
        let mut cleared = (*decode_documents(&adopted).unwrap()).clone();
        cleared.lifecycle = Arc::default();
        assert_eq!(encode_documents(Arc::new(cleared)).unwrap(), edited);
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), edited);
    }
}

/// Positional JSON arrays are refused at every level of assertion files, lifecycle.json and MCP policies.
#[test]
fn positional_json_arrays_are_refused_in_assertions_assessments_and_policy() {
    let (mut session, _, draft, _) = placed();
    let standard = Policy::standard();
    let base: Value = serde_json::from_str(&draft.encode().unwrap()).unwrap();
    let positional_assertion =
        json!([FORMAT, &base["address"], &base["target"], &base["scheme"], &base["state"], &base["reason"], &base["provenance"]]);
    assert!(Assertion::read(&positional_assertion.to_string()).unwrap_err().contains("expected an object"));
    for (field, array) in [
        ("address", json!([draft.address.library, draft.address.id, draft.address.revision])),
        ("target", json!([draft.target.library, "symbol", draft.target.id, draft.target.revision])),
        ("scheme", json!([draft.scheme.library, draft.scheme.id, draft.scheme.revision])),
        ("provenance", json!([draft.provenance.publisher])),
    ] {
        let mut mutated = base.clone();
        mutated[field] = array;
        assert!(Assertion::read(&mutated.to_string()).unwrap_err().contains("expected an object"), "nested {field}");
    }

    let positional_stored = json!([[&base], [&standard]]);
    let positional_assertion_item = json!({"assertions":[&positional_assertion],"policies":[]});
    let positional_policy_item = json!({
        "assertions":[&base],
        "policies":[[&standard.scheme, [["draft", "Draft", "draft"]]]]
    });
    for (label, value) in [("stored", positional_stored), ("assertion item", positional_assertion_item), ("policy item", positional_policy_item)] {
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(Assessments::decode(&bytes).unwrap_err().contains("expected an object"), "{label}");
    }

    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let before_history = history_state(&session);
    for (label, policy) in [
        ("policy", json!([&standard.scheme, [{"id":"draft","label":"Draft","disposition":"draft"}]])),
        (
            "policy.scheme",
            json!({"scheme":[&standard.scheme.library, &standard.scheme.id, standard.scheme.revision],"states":[{"id":"draft","label":"Draft","disposition":"draft"}]}),
        ),
        ("policy.states[0]", json!({"scheme":&standard.scheme,"states":[["draft","Draft","draft"]]})),
    ] {
        let mut request = params(&session, &draft);
        request["policy"] = policy;
        assert!(session.execute(ADOPT, &request).is_err(), "{label}");
        assert_eq!(history_state(&session), before_history, "{label}");
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before, "{label}");
    }
}

/// Build a native source with `count` distinct symbols and assertions (each on its own scheme revision).
fn bulk_source(count: usize, reason: &str) -> (Documents, Vec<Assertion>) {
    let (base_docs, template, _) = source();
    let symbol_path = base_docs.keys().find(|path| path.starts_with("symbols/")).unwrap().clone();
    let symbol_value: Value = serde_json::from_str(&base_docs[&symbol_path]).unwrap();
    let mut documents = Documents::new();
    documents.insert("library.json".into(), base_docs["library.json"].clone());
    let mut assertions = Vec::with_capacity(count);
    for index in 0..count {
        let id = format!("sym-{index}");
        let mut sym = symbol_value.clone();
        sym["id"] = json!(id);
        documents.insert(format!("symbols/{id}.json"), sym.to_string());
        let mut assertion = template.clone();
        assertion.address.id = format!("assess-{index}");
        assertion.address.revision = 1;
        assertion.target.id = id;
        assertion.scheme = Reference { library: "lifecycle-demo".into(), id: "scheme".into(), revision: (index + 1) as u32 };
        assertion.reason = reason.into();
        documents.insert(format!("lifecycle-assertions/assess-{index}.r1.json"), assertion.encode().unwrap());
        assertions.push(assertion);
    }
    (documents, assertions)
}

/// Boundary tests for 128/129 assessments, 32/33 policies, and the 4 MiB document size limit,
/// including dry-run parity and unchanged revision/history/bytes on every refusal.
#[test]
fn assessment_and_policy_count_and_size_limits_refuse_cleanly() {
    // 1. 128 assessments accepted, 129th refused (apply and dry_run).
    let (documents, assertions) = bulk_source(129, "Pending review");
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
    for assertion in &assertions[..128] {
        session.execute(ADOPT, &params(&session, assertion)).unwrap();
    }
    let at_limit = encode_documents(session.snapshot().unwrap()).unwrap();
    let at_limit_history = history_state(&session);
    for dry_run in [true, false] {
        let mut over = params(&session, &assertions[128]);
        over["dry_run"] = json!(dry_run);
        assert!(session.execute(ADOPT, &over).unwrap_err().to_string().contains("up to 128 lifecycle assessments"));
        assert_eq!(history_state(&session), at_limit_history);
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), at_limit);
    }

    // 2. 32 policies accepted, 33rd refused (apply and dry_run).
    let (policy_docs, policy_assertions) = bulk_source(33, "Policy check");
    let mut policy_session = Session::default();
    policy_session.apply("project.new", &json!({})).unwrap();
    policy_session.execute(LOAD, &json!({"source":"fixture","documents":policy_docs})).unwrap();
    for assertion in &policy_assertions[..32] {
        let mut req = params(&policy_session, assertion);
        req["policy"] = json!(Policy {
            scheme: assertion.scheme.clone(),
            states: vec![State { id: "draft".into(), label: "Draft".into(), disposition: Disposition::Draft }],
        });
        policy_session.execute(ADOPT, &req).unwrap();
    }
    let at_policy_limit = encode_documents(policy_session.snapshot().unwrap()).unwrap();
    let at_policy_history = history_state(&policy_session);
    for dry_run in [true, false] {
        let mut over = params(&policy_session, &policy_assertions[32]);
        over["dry_run"] = json!(dry_run);
        over["policy"] = json!(Policy {
            scheme: policy_assertions[32].scheme.clone(),
            states: vec![State { id: "draft".into(), label: "Draft".into(), disposition: Disposition::Draft }],
        });
        assert!(policy_session.execute(ADOPT, &over).unwrap_err().to_string().contains("up to 32 lifecycle policies"));
        assert_eq!(history_state(&policy_session), at_policy_history);
        assert_eq!(encode_documents(policy_session.snapshot().unwrap()).unwrap(), at_policy_limit);
    }

    // 3. Exact 4 MiB accepted vs 4 MiB + 1 refused in decode, and ADOPT exceeding 4 MiB refused cleanly.
    const FOUR_MIB: usize = 4 * 1024 * 1024;
    let (_, draft, _) = source();
    let standard = Policy::standard();
    let mut padded = stored(&[&draft], &[&standard]);
    padded.resize(FOUR_MIB, b' ');
    assert!(Assessments::decode(&padded).is_ok());
    padded.push(b' ');
    assert!(Assessments::decode(&padded).unwrap_err().contains("exceeds 4 MiB"));

    // 128 max-character 4-byte UTF-8 assertions (~2.1 MiB) + 32 max-state policies (~2.1 MiB) cross 4 MiB.
    let big_reason = "😀".repeat(MAX_NOTE_CHARS);
    let big_label = "😀".repeat(MAX_LABEL_CHARS);
    let (big_docs, big_assertions) = bulk_source(128, &big_reason);
    let mut size_session = Session::default();
    size_session.apply("project.new", &json!({})).unwrap();
    size_session.execute(LOAD, &json!({"source":"fixture","documents":big_docs})).unwrap();
    let mut hit_size_limit = false;
    for (index, assertion) in big_assertions.iter().enumerate() {
        let mut req = params(&size_session, assertion);
        if index < 32 {
            let states = (0..MAX_STATES)
                .map(|i| State {
                    id: if i == 0 { "draft".into() } else { format!("s-{i}") },
                    label: big_label.clone(),
                    disposition: Disposition::Draft,
                })
                .collect();
            req["policy"] = json!(Policy { scheme: assertion.scheme.clone(), states });
        }
        let before = encode_documents(size_session.snapshot().unwrap()).unwrap();
        let before_history = history_state(&size_session);
        let mut dry = req.clone();
        dry["dry_run"] = json!(true);
        match size_session.execute(ADOPT, &dry) {
            Ok(_) => {
                assert_eq!(history_state(&size_session), before_history);
                size_session.execute(ADOPT, &req).unwrap();
            }
            Err(error) => {
                assert!(error.to_string().contains("exceeds 4 MiB"), "{error}");
                assert_eq!(history_state(&size_session), before_history);
                assert_eq!(encode_documents(size_session.snapshot().unwrap()).unwrap(), before);
                assert!(size_session.execute(ADOPT, &req).unwrap_err().to_string().contains("exceeds 4 MiB"));
                assert_eq!(history_state(&size_session), before_history);
                assert_eq!(encode_documents(size_session.snapshot().unwrap()).unwrap(), before);
                hit_size_limit = true;
                break;
            }
        }
    }
    assert!(hit_size_limit, "expected 4 MiB lifecycle document limit refusal");
}

/// Pre-folder projects (/6, /9 and /13) can adopt a lifecycle assessment, write /64 and reopen cleanly.
#[test]
fn pre_folder_projects_adopt_lifecycle_and_reopen_project_64() {
    let base: crate::persistence::Documents = [
        (crate::persistence::ENTRY_POINT, include_bytes!("../../../../../app/tests/fixtures/split/two-resistors/project.json").as_slice()),
        ("board.json", include_bytes!("../../../../../app/tests/fixtures/split/two-resistors/board.json").as_slice()),
        (
            "sheets/0bbcf5e3-dd21-47b3-ae6b-bd66c8f97989.json",
            include_bytes!("../../../../../app/tests/fixtures/split/two-resistors/sheets/0bbcf5e3-dd21-47b3-ae6b-bd66c8f97989.json").as_slice(),
        ),
        (
            "library/symbols/starter/resistor.r1.json",
            include_bytes!("../../../../../app/tests/fixtures/split/two-resistors/library/symbols/starter/resistor.r1.json").as_slice(),
        ),
    ]
    .into_iter()
    .map(|(path, bytes)| (path.to_owned(), bytes.to_vec()))
    .collect();

    let project_6 = edit_project_header(&base, |header| header["format"] = json!("pcbcraft-project/6"));
    let mut project_9 = edit_project_header(&base, |header| {
        header["format"] = json!("pcbcraft-project/9");
        header["numeric_id_high_water"] = json!(4);
        header["boards"] = json!([]);
    });
    project_9.remove("board.json");

    // A boardless project/9 pins a package for component 2 (R1) as a schematic footprint choice,
    // which selects project/13. The command addresses the component, not a sheet symbol, and
    // resolves only exact catalog footprints such as basic-components/resistor-0402 r4.
    let mut prep_13 = Session::default();
    prep_13.open_snapshot(decode_documents(&project_9).unwrap());
    prep_13
        .apply(
            crate::board::assignment::ASSIGN_FOOTPRINT,
            &json!({
                "component": "component-2",
                "library": "basic-components",
                "definition": "resistor-0402",
                "definition_revision": 4,
                "mapping": {"1": "1", "2": "2"},
                "expected_revision": prep_13.revision(),
            }),
        )
        .unwrap();
    let project_13 = encode_documents(prep_13.snapshot().unwrap()).unwrap();
    let entry_13: Value = serde_json::from_slice(&project_13[crate::persistence::ENTRY_POINT]).unwrap();
    assert_eq!(entry_13["format"], "pcbcraft-project/13");
    assert_eq!(entry_13["footprint_choices"][0]["component"], 2);

    let (documents, draft, _) = source();
    for (label, fixture) in [("project/6", project_6), ("project/9", project_9), ("project/13", project_13)] {
        let mut session = Session::default();
        session.open_snapshot(decode_documents(&fixture).unwrap());
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
        session.execute(ADOPT, &params(&session, &draft)).unwrap();
        let saved = encode_documents(session.snapshot().unwrap()).unwrap();
        let entry: Value = serde_json::from_slice(&saved[crate::persistence::ENTRY_POINT]).unwrap();
        assert_eq!(entry["format"], "pcbcraft-project/64", "{label}");
        assert_eq!(entry.get("footprint_choices").is_some(), label == "project/13", "{label}");
        assert!(saved.contains_key("library/lifecycle.json"), "{label}");
        let reopened = decode_documents(&saved).unwrap();
        assert_eq!(reopened.lifecycle().disposition(&draft.target), Some(Disposition::Draft), "{label}");
        assert_eq!(encode_documents(reopened).unwrap(), saved, "{label}");
        session.apply("edit.undo", &Value::Null).unwrap();
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before, "{label}");
    }
}

/// Retaining assertion history per registration ensures that a refresh omitting a revision cannot
/// erase its previously seen content: re-introducing the same content succeeds, re-introducing
/// mutated content quarantines the address, and explicit removal clears the history.
#[test]
fn refresh_omitting_assertion_retains_history_and_quarantines_mutated_reintroduction() {
    let (documents, draft, released) = source();
    let mut omitted = documents.clone();
    assert!(omitted.remove("lifecycle-assertions/drawing-assessment.r1.json").is_some());

    let mut mutated_assertion = draft.clone();
    mutated_assertion.reason = "Silently altered after omission".into();
    let mut mutated = documents.clone();
    mutated.insert("lifecycle-assertions/drawing-assessment.r1.json".into(), mutated_assertion.encode().unwrap());

    let mut catalog = Catalog::default();
    catalog.install("fixture", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    assert_eq!(catalog.lifecycle_assertion(&draft.address).unwrap().as_ref(), &draft);
    assert_eq!(catalog.lifecycle_assertion(&released.address).unwrap().as_ref(), &released);

    // Refresh omitting r1 hides r1 while retaining its seen history for future refreshes.
    catalog.install("fixture", Arc::new(NativeSource::load(&omitted).unwrap())).unwrap();
    assert_eq!(catalog.lifecycle_assertion(&draft.address).unwrap_err(), "Lifecycle assertion revision is unavailable");
    assert_eq!(catalog.lifecycle_assertion(&released.address).unwrap().as_ref(), &released);

    // Re-introducing identical content for r1 restores availability without false conflict.
    catalog.install("fixture", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    assert_eq!(catalog.lifecycle_assertion(&draft.address).unwrap().as_ref(), &draft);

    // Omitting r1 and then re-introducing mutated content quarantines the address.
    catalog.install("fixture", Arc::new(NativeSource::load(&omitted).unwrap())).unwrap();
    catalog.install("fixture", Arc::new(NativeSource::load(&mutated).unwrap())).unwrap();
    assert_eq!(catalog.lifecycle_assertion(&draft.address).unwrap_err(), "Conflicting content at this immutable lifecycle assertion address");
    assert_eq!(catalog.lifecycle_assertion(&released.address).unwrap().as_ref(), &released);

    // Explicit removal clears retained assertion history for the registration.
    catalog.remove("fixture").unwrap();
    catalog.install("fixture", Arc::new(NativeSource::load(&mutated).unwrap())).unwrap();
    assert_eq!(catalog.lifecycle_assertion(&draft.address).unwrap().as_ref(), &mutated_assertion);
}
