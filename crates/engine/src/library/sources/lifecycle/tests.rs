use super::*;
use crate::{
    PLACE_SYMBOL, Session,
    authoring::{Item, Kind as DraftKind, publishing},
    library::sources::{Documents, Kind, LOAD, symbol_digest},
    persistence::{decode_documents, encode_documents},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Build the target through the native authoring encoder and assertions through their own codec.
fn source() -> (Documents, Assertion, Assertion) {
    let manifest = publishing::new_library("lifecycle-demo", "Lifecycle demo", "MIT OR Apache-2.0").unwrap();
    let native = publishing::prepare(Arc::new(Item::new(DraftKind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
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
    assert_eq!(entry["format"], "pcbcraft-project/52");
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
    let changed = session.execute(ADOPT, &params(&session, &released)).unwrap();
    assert_eq!(changed["before"]["revision"], 1);
    assert_eq!(changed["after"]["revision"], 2);
    assert_eq!(changed["disposition_after"], "released");
    assert_eq!(session.revision(), pre_revision + 2);
    assert_eq!(session.execute(ADOPT, &params(&session, &released)).unwrap()["changed"], false);
    let released_files = encode_documents(session.snapshot().unwrap()).unwrap();
    for (path, bytes) in &starting {
        assert_eq!(&released_files[path], bytes, "unrelated document {path}");
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

/// Publisher refresh can expose more information but may not change a pin or overwrite immutable assertion bytes.
#[test]
fn conflict_and_stale_selection_do_not_replace_a_pinned_assertion() {
    let (mut session, mut documents, draft, mut released) = placed();
    let mut stale = params(&session, &released);
    session.apply(ADOPT, &params(&session, &draft)).unwrap();
    assert_eq!(session.execute(LOOKUP, &params_for_lookup(&released)).unwrap()["digest"], released.digest().unwrap());
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(session.execute(ADOPT, &stale).unwrap_err().to_string().contains("project changed"));
    stale["expected_revision"] = json!(session.revision());
    stale["digest"] = json!("0".repeat(64));
    assert!(session.execute(ADOPT, &stale).unwrap_err().to_string().contains("content changed"));
    released.state = "obsolete".into();
    documents.insert("lifecycle-assertions/drawing-assessment.r2.json".into(), released.encode().unwrap());
    session.execute(LOAD, &json!({"source":"fixture","documents":documents})).unwrap();
    assert!(session.execute(LOOKUP, &params_for_lookup(&released)).unwrap_err().to_string().contains("Conflicting"));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    assert_eq!(session.snapshot().unwrap().lifecycle().disposition(&draft.target), Some(Disposition::Draft));
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
    policy.scheme.revision = 1;
    edit["policy"] = json!(policy);
    assert!(session.execute(ADOPT, &edit).unwrap_err().to_string().contains("exact scheme revision"));
    policy.scheme.revision = 2;
    edit["policy"] = json!(policy);
    assert_eq!(session.execute(ADOPT, &edit).unwrap()["disposition_after"], "deprecated");
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(session.snapshot().unwrap().lifecycle().disposition(&draft.target), None);
}

/// Refuse invalid publisher documents and policy duplicates instead of interpreting malformed metadata.
#[test]
fn codecs_refuse_wrong_versions_unknown_fields_and_limits() {
    let (_, mut draft, _) = source();
    draft.reason = "é".repeat(MAX_NOTE_BYTES / 2);
    let encoded = draft.encode().unwrap();
    assert_eq!(Assertion::read(&encoded).unwrap(), draft);
    draft.reason.push('x');
    assert!(draft.encode().unwrap_err().contains("exceeds 4096 UTF-8 bytes"));
    let mut value: Value = serde_json::from_str(&encoded).unwrap();
    value["target"]["unexpected"] = json!(true);
    assert!(Assertion::read(&value.to_string()).unwrap_err().contains("unknown field `unexpected`"));
    value = serde_json::from_str(&encoded).unwrap();
    value["format"] = json!("pcbcraft-lifecycle-assertion/2");
    assert!(Assertion::read(&value.to_string()).unwrap_err().contains("Unsupported lifecycle assertion format"));
    let mut policy = Policy::standard();
    policy.states = (0..MAX_STATES)
        .map(|i| State { id: format!("state-{i}"), label: "é".repeat(MAX_LABEL_BYTES / 2), disposition: Disposition::Draft })
        .collect();
    assert!(policy.validate().is_ok());
    policy.states.push(State { id: "extra".into(), label: "Extra".into(), disposition: Disposition::Draft });
    assert!(policy.validate().unwrap_err().contains("must contain 1 to 64 states"));
    policy.states.truncate(2);
    policy.states[1].id = policy.states[0].id.clone();
    assert!(policy.validate().unwrap_err().contains("Duplicate lifecycle state ID"));
}

/// A lifecycle document requires the new header and a nonempty valid assessment set.
#[test]
fn lifecycle_file_and_project_header_must_agree() {
    let (mut session, _, draft, _) = placed();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    session.apply(ADOPT, &params(&session, &draft)).unwrap();
    let after = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut missing = after.clone();
    missing.remove("library/lifecycle.json");
    assert_eq!(missing, before);
    assert!(decode_documents(&missing).unwrap().lifecycle().is_empty());
    let orphan = edit_project_header(&after, |header| header["format"] = json!("pcbcraft-project/16"));
    assert!(decode_documents(&orphan).unwrap_err().contains("requires project/52"));
    let mut empty = after.clone();
    empty.insert("library/lifecycle.json".into(), br#"{"assertions":[],"policies":[]}"#.to_vec());
    assert!(decode_documents(&empty).unwrap_err().contains("assessment or policy count"));
    let overlay = edit_project_header(&after, |header| header["content_format"] = json!("pcbcraft-project/16"));
    assert!(decode_documents(&overlay).unwrap_err().contains("unknown field `content_format`"));
    let unshipped = edit_project_header(&before, |header| header["format"] = json!("pcbcraft-project/47"));
    assert!(decode_documents(&unshipped).unwrap_err().contains("unsupported project format pcbcraft-project/47"));
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
    let error = reopened.execute(ADOPT, &params(&reopened, &changed)).unwrap_err().to_string();
    assert!(error.contains("already has different pinned content"), "{error}");
    assert_eq!(encode_documents(reopened.snapshot().unwrap()).unwrap(), saved);
    assert_eq!(reopened.snapshot().unwrap().lifecycle().disposition(&draft.target), Some(Disposition::Draft));
}
