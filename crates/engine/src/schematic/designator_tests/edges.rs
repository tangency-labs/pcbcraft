//! Refusal and native-byte boundaries use command-generated locked projects.
use super::*;
use crate::schematic::fields::{Field, Metadata};
use std::collections::BTreeMap;

/// Capture a valid locked flat fragment or a closed child tree through the shared command.
fn locked_clipboard(tree: bool) -> (Session, Value) {
    let mut source = fixture(false);
    lock(&mut source, true);
    let root = source.project().unwrap().sheet_target().id();
    let mut args = json!({"sheet":root,"items":{"symbols":["symbol-2"],"wires":[]}});
    if tree {
        edit(&mut source, crate::hierarchy::CREATE_CHILD, json!({"sheet":root,"name":"Child","x_nm":0,"y_nm":0}));
        args["objects"] = json!([format!("sheet-symbol-{}", source.project().unwrap().subsheets[0].id)]);
    }
    args["expected_revision"] = json!(source.revision());
    let response = source.execute(clipboard::COPY, &args).unwrap();
    let envelope: Value = serde_json::from_str(response["clipboard"].as_str().unwrap()).unwrap();
    assert_eq!(envelope["format"], "pcbcraft-clipboard/3");
    let entry: Value = serde_json::from_str(envelope["documents"][persistence::ENTRY_POINT].as_str().unwrap()).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/52");
    assert!(clipboard::Fragment::from_text(&envelope.to_string()).is_ok());
    (source, envelope)
}

/// Carry valid production documents into a deliberately out-of-scope clipboard envelope.
fn replace_documents(envelope: &mut Value, source: &Session) {
    let native = documents(source);
    persistence::decode_documents(&native).unwrap();
    let text: BTreeMap<_, _> = native.into_iter().map(|(path, bytes)| (path, String::from_utf8(bytes).unwrap())).collect();
    envelope["documents"] = serde_json::to_value(text).unwrap();
}

/// Both decode and Paste must reach the named scope/root refusal at fresh revision authority.
fn refuse(source: &Session, destination: &mut Session, envelope: &Value, reason: &str) {
    let source_snapshot = source.snapshot().unwrap();
    let source_history = (source.revision(), source.undo.len(), source.redo.len());
    let source_bytes = documents(source);
    let snapshot = destination.snapshot().unwrap();
    let history = (destination.revision(), destination.undo.len(), destination.redo.len());
    let before = documents(destination);
    let text = envelope.to_string();
    let error = clipboard::Fragment::from_text(&text).unwrap_err();
    assert!(matches!(&error, CommandError::InvalidParams(message) if message.contains(reason)), "{error}");
    let error = destination.apply(clipboard::PASTE, &json!({"text":text,"x_nm":0,"y_nm":0,"expected_revision":destination.revision()})).unwrap_err();
    assert!(matches!(&error, CommandError::InvalidParams(message) if message.contains(reason)), "{error}");
    assert!(Arc::ptr_eq(&snapshot, &destination.snapshot().unwrap()));
    assert_eq!((destination.revision(), destination.undo.len(), destination.redo.len()), history);
    assert_eq!(documents(destination), before);
    assert!(Arc::ptr_eq(&source_snapshot, &source.snapshot().unwrap()));
    assert_eq!((source.revision(), source.undo.len(), source.redo.len()), source_history);
    assert_eq!(documents(source), source_bytes);
}

/// Format 3 inherits root and ownership checks after successful lock-bearing native decoding.
/// Null and omission select flat scope; neither can disguise a hierarchy as a flat fragment.
#[test]
fn designator_lock_clipboard_v3_root_scope_and_envelope_refusals() {
    let mut destination = fixture(false);
    for tree in [false, true] {
        let (source, envelope) = locked_clipboard(tree);
        let mut control = fixture(false);
        edit(&mut control, clipboard::PASTE, json!({"text":envelope.to_string(),"x_nm":50_800_000,"y_nm":0}));
        assert!(control.project().unwrap().components().any(|part| part.designator_locked()));
        for (field, value) in [("kind", json!("board")), ("format", json!("pcbcraft-clipboard/999"))] {
            let mut bad = envelope.clone();
            bad[field] = value;
            refuse(&source, &mut destination, &bad, "Unsupported clipboard format or editor kind");
        }
        for (value, reason) in
            [(json!(uuid::Uuid::nil()), "explicit root sheet UUID"), (json!(uuid::Uuid::from_u128(1)), "root sheet is unavailable")]
        {
            let mut bad = envelope.clone();
            bad["root_sheet"] = value;
            refuse(&source, &mut destination, &bad, reason);
        }
        let mut absent = envelope.clone();
        absent.as_object_mut().unwrap().remove("root_sheet");
        let mut null = envelope.clone();
        null["root_sheet"] = Value::Null;
        if tree {
            for bad in [absent, null] {
                refuse(&source, &mut destination, &bad, "nonempty schematic fragment of the declared version");
            }
        } else {
            for flat in [absent, null] {
                assert!(clipboard::Fragment::from_text(&flat.to_string()).is_ok());
                let mut target = fixture(false);
                edit(&mut target, clipboard::PASTE, json!({"text":flat.to_string(),"x_nm":50_800_000,"y_nm":0}));
            }
            let mut disguised = envelope.clone();
            disguised["root_sheet"] = json!(source.project().unwrap().sheet_target().id());
            refuse(&source, &mut destination, &disguised, "nonempty schematic fragment of the declared version");
        }
    }
    let (_, mut envelope) = locked_clipboard(false);
    let mut board = fixture(true);
    lock(&mut board, true);
    replace_documents(&mut envelope, &board);
    refuse(&board, &mut destination, &envelope, "nonempty schematic fragment of the declared version");
    let mut orphan = fixture(false);
    lock(&mut orphan, true);
    let root = orphan.project().unwrap().sheet_target().id();
    edit(&mut orphan, crate::sheets::CREATE_SHEET, json!({"name":"Unselected root"}));
    replace_documents(&mut envelope, &orphan);
    envelope["root_sheet"] = json!(root);
    refuse(&orphan, &mut destination, &envelope, "outside its selected sheet subtree");
}

/// Fill bounded optional fields with exact UTF-8 byte counts, without exceeding character limits.
fn metadata(bytes: usize) -> Metadata {
    let mut remaining = bytes;
    let mut result = Metadata::default();
    for index in 0..64 {
        let count = remaining.min(8192);
        let mut value = "🦅".repeat(count / 4);
        match count % 4 {
            1 => value.push('a'),
            2 => value.push('é'),
            3 => value.push('€'),
            _ => {}
        }
        remaining -= count;
        result.fields.insert(uuid::Uuid::from_u128(index + 1).to_string(), Field { name: format!("Field {index}"), value });
    }
    assert_eq!(remaining, 0);
    result.validate().unwrap();
    result
}

/// Replace one instance's metadata through the same property transaction used for its lock.
fn set_metadata(session: &mut Session, symbol: &str, metadata: &Metadata) {
    let project = session.project().unwrap();
    let placement = project.symbols().iter().find(|placement| placement.id() == symbol).unwrap();
    let part = project.component(placement).unwrap();
    let params = json!({"symbol":symbol,"reference":part.reference(),"value":part.value(),"metadata":metadata});
    edit(session, SET_SYMBOL_FIELDS, params);
}

/// Native size is independent of model validity: preview refuses cap+1 without publication,
/// ordinary Apply can publish it, and stale/invalid requests still get their ordinary causes.
#[test]
fn designator_lock_real_document_limit_and_authority_precedence() {
    let mut session = fixture(false);
    for index in 2..32 {
        edit(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":index*25_400_000i64,"y_nm":0}));
    }
    let symbols: Vec<_> = session.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    for symbol in &symbols[..31] {
        set_metadata(&mut session, symbol, &metadata(64 * 8192));
        let mut fresh = Session { libraries: session.libraries.clone(), ..Session::default() };
        fresh.open_snapshot(session.snapshot().unwrap());
        session = fresh;
    }
    let last = &symbols[31];
    set_metadata(&mut session, last, &metadata(0));
    let base_length = documents(&session)[persistence::ENTRY_POINT].len();
    let mut calibrated = Session { libraries: session.libraries.clone(), ..Session::default() };
    calibrated.open_snapshot(session.snapshot().unwrap());
    lock(&mut calibrated, true);
    let lock_bytes = documents(&calibrated)[persistence::ENTRY_POINT].len() - base_length;
    drop(calibrated);
    let payload = persistence::MAX_DOCUMENT_BYTES.checked_sub(base_length + lock_bytes).unwrap();
    set_metadata(&mut session, last, &metadata(payload));
    assert_eq!(documents(&session)[persistence::ENTRY_POINT].len(), persistence::MAX_DOCUMENT_BYTES - lock_bytes);
    let params =
        |revision| json!({"symbol":"symbol-2","reference":"R1","value":"R","designator_locked":true,"dry_run":true,"expected_revision":revision});
    let good = session.execute(SET_SYMBOL_FIELDS, &params(session.revision())).unwrap();
    assert_eq!(good["changes"][0]["after"].as_str().unwrap().len(), persistence::MAX_DOCUMENT_BYTES);
    lock(&mut session, true);
    assert_eq!(documents(&session)[persistence::ENTRY_POINT].len(), persistence::MAX_DOCUMENT_BYTES);
    edit(&mut session, "edit.undo", Value::Null);
    set_metadata(&mut session, last, &metadata(payload + 1));
    assert_eq!(documents(&session)[persistence::ENTRY_POINT].len(), persistence::MAX_DOCUMENT_BYTES - lock_bytes + 1);
    let before = session.snapshot().unwrap();
    let history = (session.revision(), session.undo.len(), session.redo.len());
    let error = session.execute(SET_SYMBOL_FIELDS, &params(session.revision())).unwrap_err();
    assert!(matches!(&error, CommandError::InvalidParams(message) if message == "Encoded instance document exceeds its size limit."), "{error}");
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!((session.revision(), session.undo.len(), session.redo.len()), history);
    lock(&mut session, true);
    assert_eq!(persistence::encode_documents(session.snapshot().unwrap()).unwrap_err(), "Encoded instance document exceeds its size limit.");
    let oversized = session.snapshot().unwrap();
    let history = (session.revision(), session.undo.len(), session.redo.len());
    let error = session.execute(SET_SYMBOL_FIELDS, &params(0)).unwrap_err();
    assert_eq!(error, CommandError::StaleRevision { current: history.0 });
    let mut invalid = params(session.revision());
    invalid["reference"] = json!("R 1");
    let error = session.execute(SET_SYMBOL_FIELDS, &invalid).unwrap_err();
    assert!(matches!(&error, CommandError::InvalidParams(message) if message.contains("reference") && message.contains("spaces")), "{error}");
    assert!(Arc::ptr_eq(&oversized, &session.snapshot().unwrap()));
    assert_eq!((session.revision(), session.undo.len(), session.redo.len()), history);
}

/// Adopting a genuinely flat legacy board reports both added and removed native paths as null.
#[test]
fn designator_lock_preview_reports_both_absent_document_sides() {
    let mut session = fixture(true);
    let mut historical = crate::Project::clone(&persistence::diagnostic_snapshot(session.snapshot().unwrap()));
    historical.layout = None;
    historical.numeric_id_high_water = None;
    Arc::make_mut(&mut historical.board).ownership = crate::board::identity::Ownership::Legacy;
    let native = persistence::encode_documents(Arc::new(historical)).unwrap();
    session.open_snapshot(persistence::decode_documents(&native).unwrap());
    let before = documents(&session);
    let preview = session
        .execute(
            SET_SYMBOL_FIELDS,
            &json!({"symbol":"symbol-2","reference":"R1","value":"R","designator_locked":true,"dry_run":true,"expected_revision":session.revision()}),
        )
        .unwrap();
    assert_eq!(documents(&session), before);
    lock(&mut session, true);
    let after = documents(&session);
    let changes = preview["changes"].as_array().unwrap();
    assert!(changes.iter().any(|change| change["before"].is_null()));
    assert!(changes.iter().any(|change| change["after"].is_null()));
    let paths: std::collections::BTreeSet<_> = before.keys().chain(after.keys()).collect();
    let expected: Vec<_> = paths.into_iter().filter(|path| before.get(*path) != after.get(*path)).map(|path| {
        json!({"path":path,"before":before.get(path).map(|bytes| std::str::from_utf8(bytes).unwrap()),"after":after.get(path).map(|bytes| std::str::from_utf8(bytes).unwrap())})
    }).collect();
    assert_eq!(preview["changes"], json!(expected));
}
