//! Both property presentations share instance lock intent and the editor's revision authority.
use super::*;
use pcbcraft_engine::persistence::{decode_documents, encode_documents};

/// Select a command-placed part with Properties visible, without relying on camera coordinates.
fn fixture(width: f32, dark: bool) -> Harness<'static, PcbApp> {
    let mut h = harness();
    resistor_at_origin(&mut h);
    h.set_size(egui::vec2(width, 900.0));
    h.ctx.set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
    h.state_mut().properties_open = true;
    h.state_mut().views.active.schematic.selection = Selection::one_symbol("symbol-2".into());
    h.run();
    h
}

/// Read the authoritative flag, not the checkbox draft.
fn locked(h: &Harness<'_, PcbApp>) -> bool {
    h.state().session.project().unwrap().components().next().unwrap().designator_locked()
}

/// Compact and expanded controls keep drafts local until Apply/OK; both write the same
/// saved lock, and one Undo restores the original bytes. Cancel discards a draft.
#[test]
fn designator_lock_properties_share_command_undo_and_cancel() {
    for (width, dark) in [(1400.0, true), (800.0, false)] {
        let mut h = fixture(width, dark);
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        let checkbox = h.get_by_label("Lock designator").rect();
        assert!(checkbox.width() > 20.0 && checkbox.right() <= width && checkbox.bottom() <= 900.0);
        click(&mut h, "Lock designator");
        assert!(!locked(&h));
        assert_eq!(h.state().session.revision(), revision);
        click(&mut h, "Apply changes");
        assert!(locked(&h));
        assert_eq!(h.state().session.revision(), revision + 1);
        let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        assert!(decode_documents(&saved).unwrap().components().next().unwrap().designator_locked());
        h.state_mut().run("edit.undo", &Value::Null);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        h.key_press(Key::E);
        h.run();
        click(&mut h, "Lock designator");
        assert!(!locked(&h));
        click(&mut h, "Cancel");
        assert!(!locked(&h));
        h.key_press(Key::E);
        h.run();
        click(&mut h, "Lock designator");
        click(&mut h, "OK");
        assert!(locked(&h));
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
    }
}

/// A property draft captured before an external edit cannot apply its lock to the newer state.
#[test]
fn designator_lock_properties_refuse_stale_draft() {
    let mut h = fixture(1400.0, true);
    h.key_press(Key::E);
    h.run();
    click(&mut h, "Lock designator");
    let revision = h.state().session.revision();
    h.state_mut().session.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":25_400_000,"y_nm":0,"expected_revision":revision})).unwrap();
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "OK");
    assert!(!locked(&h));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.get_by_label("View or project changed; reopen the symbol fields to continue editing.");
}

#[cfg(not(target_arch = "wasm32"))]
/// Real native folder/archive workers save and reopen the flag applied through Properties.
#[test]
fn designator_lock_native_save_and_reopen() {
    use crate::project_files::SaveFormat;
    for format in [SaveFormat::Folder, SaveFormat::Archive] {
        let directory = tempfile::tempdir().unwrap();
        let root = pcbcraft_storage::plain_path_if_equivalent(directory.path().canonicalize().unwrap());
        let path = root.join(if matches!(format, SaveFormat::Archive) { "Locked.pc" } else { "Locked" });
        let mut h = fixture(1400.0, true);
        click(&mut h, "Lock designator");
        click(&mut h, "Apply changes");
        assert!(locked(&h));
        h.state_mut().files.use_stand_in();
        let ctx = h.ctx.clone();
        h.state_mut().save_project_file(Some(format), &ctx);
        h.state_mut().files.finish_stand_in(path.clone());
        super::recent_projects::finish_file(&mut h);
        assert!(!h.state().files.dirty(&h.state().session));
        let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        assert!(path.exists());
        let saved_id = h.state().session.project().unwrap().id();
        let mut reopened = fixture(1400.0, true);
        assert!(!locked(&reopened));
        let before = reopened.state().session.snapshot().unwrap();
        assert_ne!(before.id(), saved_id);
        let revision = reopened.state().session.revision();
        let ctx = reopened.ctx.clone();
        reopened.state_mut().files.open_path(path, revision, &ctx).unwrap();
        super::recent_projects::finish_file(&mut reopened);
        let loaded = reopened.state().session.snapshot().unwrap();
        assert!(!Arc::ptr_eq(&before, &loaded));
        assert!(reopened.state().session.revision() > revision);
        assert_eq!(loaded.id(), saved_id);
        assert!(locked(&reopened));
        assert!(!loaded.components().next().unwrap().can_annotate_designator());
        assert!(!reopened.state().files.dirty(&reopened.state().session));
        assert_eq!(encode_documents(loaded).unwrap(), saved);
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Fill valid optional fields with an exact UTF-8 byte count, including a partial final field.
fn native_limit_metadata(bytes: usize) -> pcbcraft_engine::schematic::fields::Metadata {
    use pcbcraft_engine::schematic::fields::{Field, Metadata};
    let mut remaining = bytes;
    let mut metadata = Metadata::default();
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
        metadata.fields.insert(uuid::Uuid::from_u128(index + 1).to_string(), Field { name: format!("Field {index}"), value });
    }
    assert_eq!(remaining, 0);
    metadata.validate().unwrap();
    metadata
}

#[cfg(not(target_arch = "wasm32"))]
/// Apply metadata or lock intent while retaining the selected owner's reference and value.
pub(super) fn native_limit_fields(session: &mut pcbcraft_engine::Session, symbol: &str, mut params: Value) {
    let project = session.project().unwrap();
    let placement = project.symbols().iter().find(|placement| placement.id() == symbol).unwrap();
    let part = project.component(placement).unwrap();
    params["symbol"] = json!(symbol);
    params["reference"] = json!(part.reference());
    params["value"] = json!(part.value());
    params["expected_revision"] = json!(session.revision());
    session.apply(pcbcraft_engine::SET_SYMBOL_FIELDS, &params).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
/// A real folder Save rejects cap+1 before publication, retains the dirty lock and recovers
/// after Undo. Drive file completion directly rather than rendering megabytes of field text.
#[test]
fn designator_lock_native_save_limit_preserves_disk_and_recovers() {
    use crate::project_files::{ProjectFiles, SaveFormat, Update};
    use pcbcraft_engine::{Session, persistence};

    let directory = tempfile::tempdir().unwrap();
    let root = pcbcraft_storage::plain_path_if_equivalent(directory.path().canonicalize().unwrap());
    let path = root.join("Native lock limit");
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Native lock limit"})).unwrap();
    for index in 0..32 {
        session
            .apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index*25_400_000i64,"y_nm":0,"expected_revision":session.revision()}))
            .unwrap();
    }
    let symbols: Vec<_> = session.project().unwrap().symbols().iter().map(|symbol| symbol.id()).collect();
    let symbol = &symbols[0];
    native_limit_fields(&mut session, symbol, json!({"designator_locked":true}));
    native_limit_fields(&mut session, symbol, json!({"designator_locked":false}));
    for symbol in &symbols[..31] {
        native_limit_fields(&mut session, symbol, json!({"metadata":native_limit_metadata(64*8192)}));
        let snapshot = session.snapshot().unwrap();
        session = Session::default();
        session.open_snapshot(snapshot);
    }
    let last = &symbols[31];
    native_limit_fields(&mut session, last, json!({"metadata":native_limit_metadata(0)}));
    let ctx = egui::Context::default();
    let mut files = ProjectFiles::default();
    files.use_stand_in();
    files.save(&session, Some(SaveFormat::Folder), &ctx).unwrap();
    files.finish_stand_in(path.clone());
    assert!(matches!(files.poll(&mut session, &ctx), Some(Update::Message(message)) if message == "Project saved"));
    assert!(!files.dirty(&session));
    assert!(files.has_location(&session));

    let base_length = encode_documents(session.snapshot().unwrap()).unwrap()[persistence::ENTRY_POINT].len();
    let mut probe = Session::default();
    probe.open_snapshot(session.snapshot().unwrap());
    native_limit_fields(&mut probe, symbol, json!({"designator_locked":true}));
    let lock_bytes = encode_documents(probe.snapshot().unwrap()).unwrap()[persistence::ENTRY_POINT].len() - base_length;
    assert!(lock_bytes > 0);
    drop(probe);
    let payload = persistence::MAX_DOCUMENT_BYTES.checked_sub(base_length + lock_bytes).unwrap() + 1;
    native_limit_fields(&mut session, last, json!({"metadata":native_limit_metadata(payload)}));
    assert_eq!(
        encode_documents(session.snapshot().unwrap()).unwrap()[persistence::ENTRY_POINT].len() + lock_bytes,
        persistence::MAX_DOCUMENT_BYTES + 1
    );
    files.save(&session, None, &ctx).unwrap();
    files.finish_stand_in(path.clone());
    assert!(matches!(files.poll(&mut session, &ctx), Some(Update::Message(message)) if message == "Project saved"));
    assert!(!files.dirty(&session));
    let saved = pcbcraft_storage::load(&path).unwrap();
    assert_eq!(saved[persistence::ENTRY_POINT].len() + lock_bytes, persistence::MAX_DOCUMENT_BYTES + 1);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);

    native_limit_fields(&mut session, symbol, json!({"designator_locked":true}));
    let oversized = session.snapshot().unwrap();
    let placement = oversized.symbols().iter().find(|placement| placement.id() == *symbol).unwrap();
    assert!(oversized.component(placement).unwrap().designator_locked());
    let revision = session.revision();
    files.save(&session, None, &ctx).unwrap();
    files.finish_stand_in(path.clone());
    if let Some(Update::Error(error)) = files.poll(&mut session, &ctx) {
        // The schematic-only entry point closes with embedded instance records.
        assert_eq!(error, "Encoded instance document exceeds its size limit.");
    } else {
        panic!("Save did not report its encoding refusal");
    }
    assert!(!files.busy());
    assert!(files.dirty(&session));
    assert!(files.has_location(&session));
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&oversized, &session.snapshot().unwrap()));
    assert_eq!(pcbcraft_storage::load(&path).unwrap(), saved);

    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(session.project().unwrap().components().all(|part| !part.designator_locked()));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
    files.save(&session, None, &ctx).unwrap();
    files.finish_stand_in(path.clone());
    assert!(matches!(files.poll(&mut session, &ctx), Some(Update::Message(message)) if message == "Project saved"));
    assert!(!files.dirty(&session));
    assert_eq!(pcbcraft_storage::load(&path).unwrap(), saved);
}
