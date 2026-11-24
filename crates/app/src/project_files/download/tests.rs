use super::*;
use pcbcraft_engine::{
    MOVE_SYMBOL, Session,
    persistence::{
        CURRENT_BOARD_FORMAT, CURRENT_PROJECT_FORMAT, CURRENT_SHEET_FORMAT, ENTRY_POINT, decode, decode_documents, encode_documents,
        migrate_bundled_libraries,
    },
};
use serde_json::json;

/// Exercise the browser's actual incremental encoder without invoking its OS download endpoint.
fn encode(current: Arc<Project>, original: Option<Original>) -> Documents {
    let mut encoder = Encoder::new(current, original).unwrap();
    for _ in 0..10_000 {
        if let Some(documents) = encoder.step(1).unwrap() {
            return documents;
        }
    }
    panic!("bounded fixture download did not complete");
}

/// An otherwise unchanged archive with a previous entry-point version must go through the
/// incremental download writer. An unchanged download from the result can reuse its bytes.
#[test]
fn unchanged_download_advances_the_project_header() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    let current = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut previous = current.clone();
    let entry = String::from_utf8(previous[ENTRY_POINT].clone()).unwrap().replacen(CURRENT_PROJECT_FORMAT, "pcbcraft-project/16", 1);
    previous.insert(ENTRY_POINT.into(), entry.into_bytes());
    let opened = decode_documents(&previous).unwrap();
    let baseline = Original::for_project(opened.clone(), Arc::new(previous), &opened).unwrap();
    assert!(baseline.unchanged(&opened).is_none());
    assert_eq!(encode(opened, Some(baseline)), current);
    let reopened = decode_documents(&current).unwrap();
    let baseline = Original::for_project(reopened.clone(), Arc::new(current.clone()), &reopened).unwrap();
    assert_eq!(baseline.unchanged(&reopened).unwrap().as_ref(), &current);
}

/// Migration preserves unrelated formatting but must never restore changed sheet/library addresses.
#[test]
fn migrated_download_reuses_only_logically_unchanged_documents() {
    for current_layout in [false, true] {
        let snapshot = decode(include_bytes!("../../../tests/fixtures/two-resistors.pc")).unwrap();
        let snapshot = if current_layout { Arc::new(snapshot.with_board_folders(".").unwrap().with_sheet_occurrences().unwrap()) } else { snapshot };
        let mut original = encode_documents(snapshot).unwrap();
        for bytes in original.values_mut() {
            bytes.extend_from_slice(b"\n \t");
        }
        let saved = decode_documents(&original).unwrap();
        let current = migrate_bundled_libraries(saved.clone()).unwrap();
        assert!(!Arc::ptr_eq(&saved, &current));
        let baseline = Original::for_project(saved.clone(), Arc::new(original.clone()), &current).unwrap();
        assert!(baseline.unchanged(&current).is_none(), "migrated input cannot download the old document set");
        assert_eq!(baseline.unchanged(&saved).unwrap().as_ref(), &original);
        let downloaded = encode(current.clone(), Some(baseline));
        let reopened = decode_documents(&downloaded).unwrap();
        assert!(reopened.has_sheet_occurrences());
        assert!(reopened.layout_root().is_some());
        assert!(reopened.components().all(|component| component.definition.library == "basic-components"));
        assert!(!downloaded.keys().any(|path| path.contains("/starter/")));
        let old_board = saved.board_document_path().unwrap();
        let new_board = reopened.board_document_path().unwrap();
        assert_eq!(downloaded[&new_board], original[&old_board]);
        assert_ne!(downloaded["project.json"], original["project.json"], "migrated component references must not be restored");
        for (path, bytes) in original.iter().filter(|(path, _)| path.starts_with("sheets/")) {
            assert_eq!(&downloaded[path], bytes, "unaffected unit placements retain their exact formatting");
        }
        let mut other = Session::default();
        other.apply("project.new", &json!({})).unwrap();
        assert!(Original::for_project(saved, Arc::new(original), other.project().unwrap()).is_none());
    }
}

/// Edited downloads retain the same saved baseline but write changed geometry and remain offline-openable.
#[test]
fn edited_download_keeps_unaffected_bytes_and_new_geometry() {
    let saved = migrate_bundled_libraries(decode(include_bytes!("../../../tests/fixtures/two-resistors.pc")).unwrap()).unwrap();
    let saved = Arc::new(saved.with_board_folders(".").unwrap().with_sheet_occurrences().unwrap());
    let mut original = encode_documents(saved).unwrap();
    for bytes in original.values_mut() {
        bytes.extend_from_slice(b"\n \t");
    }
    let saved = decode_documents(&original).unwrap();
    let mut session = Session::default();
    session.open_snapshot(saved.clone());
    session.apply(MOVE_SYMBOL, &json!({"symbol":"symbol-2","x_nm":1270000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let current = session.snapshot().unwrap();
    let baseline = Original::for_project(saved, Arc::new(original.clone()), &current).unwrap();
    let downloaded = encode(current.clone(), Some(baseline));
    assert_eq!(downloaded["project.json"], original["project.json"]);
    let changed: Vec<_> = original.keys().filter(|path| original[*path] != downloaded[*path]).collect();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].starts_with("sheets/"));
    assert_eq!(encode_documents(decode_documents(&downloaded).unwrap()).unwrap(), encode_documents(current).unwrap());
}

/// A papered legacy sheet is rewritten by the incremental browser download even when
/// no in-memory edits or paper adoption were needed.
#[test]
fn unchanged_download_advances_the_sheet_header() {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let snapshot = session.snapshot().unwrap();
    let path = format!("sheets/{}.json", snapshot.sheet_target().id());
    let current = encode_documents(snapshot).unwrap();
    let mut previous = current.clone();
    let historical = String::from_utf8(current[&path].clone()).unwrap().replacen(CURRENT_SHEET_FORMAT, "pcbcraft-sheet/9", 1);
    previous.insert(path, historical.into_bytes());
    let opened = decode_documents(&previous).unwrap();
    let baseline = Original::for_project(opened.clone(), Arc::new(previous), &opened).unwrap();
    assert!(baseline.unchanged(&opened).is_none());
    let migrated = encode(opened, Some(baseline));
    assert_eq!(migrated, current);
    let reopened = decode_documents(&migrated).unwrap();
    let baseline = Original::for_project(reopened.clone(), Arc::new(migrated), &reopened).unwrap();
    assert_eq!(baseline.unchanged(&reopened).as_deref(), Some(&current));
}

/// Browser downloads migrate a historical board with or without an unrelated edit,
/// and subsequent exports keep the current format when groups are added.
#[test]
fn legacy_board_download_migrates_and_groups_keep_the_current_format() {
    let saved = migrate_bundled_libraries(decode(include_bytes!("../../../tests/fixtures/two-resistors.pc")).unwrap()).unwrap();
    let saved = Arc::new(saved.with_board_folders(".").unwrap().with_sheet_occurrences().unwrap());
    let current = encode_documents(saved.clone()).unwrap();
    let mut original = current.clone();
    let board_path = saved.board_document_path().unwrap();
    let old = String::from_utf8(current[&board_path].clone()).unwrap().replacen(CURRENT_BOARD_FORMAT, "pcbcraft-board/1", 1);
    original.insert(board_path.clone(), format!("{old}\n \t").into_bytes());
    let saved = decode_documents(&original).unwrap();
    let original = Arc::new(original);
    let mut session = Session::default();
    session.open_snapshot(saved.clone());
    let baseline = Original::for_project(saved.clone(), original.clone(), &saved).unwrap();
    assert!(baseline.unchanged(&saved).is_none());
    assert_eq!(encode(saved.clone(), Some(baseline)), current);
    session.apply(MOVE_SYMBOL, &json!({"symbol":"symbol-2","x_nm":1270000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let edited = session.snapshot().unwrap();
    let edited_baseline = Original::for_project(saved, original.clone(), &edited).unwrap();
    let downloaded = encode(edited.clone(), Some(edited_baseline));
    assert_eq!(downloaded[&board_path], current[&board_path]);
    assert_eq!(encode_documents(decode_documents(&downloaded).unwrap()).unwrap(), encode_documents(edited).unwrap());

    session
        .apply(
            pcbcraft_engine::INITIALIZE_BOARD_SETUP,
            &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":session.revision()}),
        )
        .unwrap();
    let top = session.project().unwrap().board_setup().unwrap().copper_stack().next().unwrap().layer;
    let mut setup = serde_json::to_value(session.project().unwrap().board_setup().unwrap()).unwrap();
    setup["stackup"]["layer_groups"] = json!([{"id":uuid::Uuid::from_u128(1),"name":"Outer","layers":[top]}]);
    session.apply(pcbcraft_engine::SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":session.revision()})).unwrap();
    let grouped = session.snapshot().unwrap();
    let grouped_download = encode(grouped.clone(), None);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&grouped_download[&board_path]).unwrap()["format"], CURRENT_BOARD_FORMAT);
    assert_eq!(encode_documents(decode_documents(&grouped_download).unwrap()).unwrap(), encode_documents(grouped).unwrap());
}
