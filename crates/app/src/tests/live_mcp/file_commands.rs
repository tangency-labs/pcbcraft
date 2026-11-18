//! `file.save_as` and `file.open` give a live client Save As and Open without the native picker,
//! through the editor's own file workers, checks and completion path (#409).

use super::*;
use crate::tests::recent_projects::finish_file;
use pcbcraft_engine::{PLACE_SYMBOL, persistence::encode_documents};

/// A display-safe canonical temporary root, as native file jobs record their paths.
fn project_directory() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = pcbcraft_storage::plain_path_if_equivalent(dir.path().canonicalize().unwrap());
    (dir, root)
}

/// The authored documents of the open project, in their saved form.
fn authored(h: &Harness<'_, PcbApp>) -> pcbcraft_engine::persistence::Documents {
    encode_documents(h.state().session().snapshot().unwrap()).unwrap()
}

/// An Untitled design saved over MCP to a new folder and a `.pc` archive reopens from each with
/// identical authored content; inspection reports each destination and result.
#[test]
fn live_save_as_and_open_round_trip_folders_and_archives() {
    let (_dir, root) = project_directory();
    let mut h = harness();
    wiring_project(&mut h);
    let (port, token) = connect(&mut h);
    let connection = (port, token.as_str());
    let folder = root.join("board");
    let archive = root.join("board-copy.pc");

    let started = window(&mut h, connection, "file.save_as", json!({"path": folder, "format": "folder"}));
    assert_eq!(started["file"]["operation"], json!({"kind": "save", "destination": folder.display().to_string()}));
    finish_file(&mut h);
    let inspected = payload(tool(&mut h, connection, "inspect_editor", json!({})));
    assert_eq!(inspected["unsaved"], false);
    assert_eq!(inspected["file"]["location"], json!({"path": folder.display().to_string(), "format": "folder"}));
    assert_eq!(inspected["file"]["last"]["status"], "succeeded");
    assert_eq!(h.state().session().project().unwrap().name, "board", "an Untitled project takes the folder's name");
    let saved = authored(&h);

    window(&mut h, connection, "file.save_as", json!({"path": root.join("board-copy"), "format": "archive"}));
    finish_file(&mut h);
    let inspected = payload(tool(&mut h, connection, "inspect_editor", json!({})));
    assert_eq!(inspected["file"]["location"], json!({"path": archive.display().to_string(), "format": "archive"}));
    assert_eq!(inspected["unsaved"], false);

    for (path, format) in [(&folder, "folder"), (&archive, "archive")] {
        let before = h.state().session().snapshot().unwrap();
        window(&mut h, connection, "file.open", json!({"path": path}));
        finish_file(&mut h);
        assert!(!Arc::ptr_eq(&before, &h.state().session().snapshot().unwrap()), "{format} was read back from disk");
        let inspected = payload(tool(&mut h, connection, "inspect_editor", json!({})));
        assert_eq!(inspected["file"]["location"], json!({"path": path.display().to_string(), "format": format}));
        assert_eq!(inspected["file"]["last"], json!({"operation": "open", "status": "succeeded", "path": path.display().to_string(), "error": null}));
        assert_eq!(inspected["unsaved"], false);
        assert_eq!(authored(&h), saved, "{format}");
    }
}

/// Refused, failed and superseded operations keep the current design, its unsaved state and its
/// undo history, and inspection reports each failure.
#[test]
fn failed_live_file_commands_preserve_the_design() {
    let (_dir, root) = project_directory();
    let mut h = harness();
    wiring_project(&mut h);
    let ctx = h.ctx.clone();
    let folder = root.join("board");
    let unchanged = |h: &Harness<'_, PcbApp>, snapshot: &Arc<pcbcraft_engine::Project>| {
        assert!(Arc::ptr_eq(snapshot, &h.state().session().snapshot().unwrap()));
        assert!(h.state().session().available("edit.undo"));
    };

    let relative = h.state_mut().run_window_command(&ctx, "file.save_as", &json!({"path": "board", "format": "folder"})).unwrap_err();
    assert!(relative.contains("absolute"), "{relative}");
    let dirty = h.state_mut().run_window_command(&ctx, "file.open", &json!({"path": folder})).unwrap_err();
    assert!(dirty.contains("unsaved changes"), "{dirty}");
    assert!(!h.state().files.busy());

    // An occupied destination fails in the worker and changes nothing.
    std::fs::write(root.join("taken.pc"), "x").unwrap();
    let before = h.state().session().snapshot().unwrap();
    h.state_mut().run_window_command(&ctx, "file.save_as", &json!({"path": root.join("taken"), "format": "archive"})).unwrap();
    finish_file(&mut h);
    let file = &window_commands::inspect_editor(h.state())["file"];
    assert_eq!(file["last"]["status"], "failed", "{file}");
    assert!(file["last"]["error"].as_str().unwrap().contains("already exists"), "{file}");
    assert_eq!(file["location"], Value::Null);
    unchanged(&h, &before);
    assert!(h.state().files.dirty(h.state().session()));

    // Saved elsewhere, then edited: a destination changed on disk refuses the in-place save.
    let folder = root.join("saved");
    h.state_mut().run_window_command(&ctx, "file.save_as", &json!({"path": folder, "format": "folder"})).unwrap();
    finish_file(&mut h);
    let revision = h.state().session().revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 25_400_000, "y_nm": 0, "expected_revision": revision}));
    h.run();
    let entry = folder.join(pcbcraft_engine::persistence::ENTRY_POINT);
    let mut changed = std::fs::read_to_string(&entry).unwrap();
    changed.push('\n');
    std::fs::write(&entry, changed).unwrap();
    let before = h.state().session().snapshot().unwrap();
    h.state_mut().run_window_command(&ctx, "file.save", &json!({})).unwrap();
    finish_file(&mut h);
    let file = &window_commands::inspect_editor(h.state())["file"];
    assert!(file["last"]["error"].as_str().unwrap().contains("changed on disk"), "{file}");
    unchanged(&h, &before);
    assert!(h.state().files.dirty(h.state().session()));

    // An open that fails, or whose project is edited while it reads, keeps the current design.
    h.state_mut().run_window_command(&ctx, "file.save_as", &json!({"path": root.join("clean"), "format": "folder"})).unwrap();
    finish_file(&mut h);
    let before = h.state().session().snapshot().unwrap();
    h.state_mut().run_window_command(&ctx, "file.open", &json!({"path": root.join("missing")})).unwrap();
    finish_file(&mut h);
    assert_eq!(window_commands::inspect_editor(h.state())["file"]["last"]["status"], "failed");
    assert!(Arc::ptr_eq(&before, &h.state().session().snapshot().unwrap()));
    h.state_mut().run_window_command(&ctx, "file.open", &json!({"path": root.join("clean")})).unwrap();
    let revision = h.state().session().revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition": "resistor", "x_nm": 0, "y_nm": 0, "expected_revision": revision}));
    let before = h.state().session().snapshot().unwrap();
    finish_file(&mut h);
    let file = &window_commands::inspect_editor(h.state())["file"];
    assert!(file["last"]["error"].as_str().unwrap().contains("Project changed while opening"), "{file}");
    unchanged(&h, &before);
}
