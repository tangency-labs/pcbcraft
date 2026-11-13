use super::*;
use pcbcraft_engine::{
    MOVE_SYMBOL, Session,
    persistence::{decode, decode_documents, encode_documents, migrate_bundled_libraries},
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
