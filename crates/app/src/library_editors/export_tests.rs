//! Whole-library export from the Parts Library runs through the editors' single file job.
#![allow(clippy::unwrap_used)]

use super::io::Outcome;
use crate::{PcbApp, tests::click};
use egui_kittest::Harness;
use serde_json::json;
use std::sync::Arc;

/// An app showing the Parts Library with the original native parts example selected.
fn app() -> PcbApp {
    let mut app = PcbApp::default();
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-parts"));
    app.session.install_library_source("example-parts", Arc::new(pcbcraft_storage::library::load(root).unwrap())).unwrap();
    app.run_window_command(&egui::Context::default(), "app.set_workspace", &json!({"workspace": "library"})).unwrap();
    app
}

/// MCP and the ribbon both reach the file job with the selected library's bundle; a held job
/// stands in for the native picker, and its result becomes the Parts Library notice.
#[test]
fn parts_library_exports_the_selected_library() {
    let mut app = app();
    let ctx = egui::Context::default();
    let view = app.views.active.id().0;
    let export = |app: &mut PcbApp, source| {
        app.run_window_command(&ctx, "app.library_source", &json!({"view": view, "action": "export", "source": source})).unwrap_err()
    };
    assert!(export(&mut app, "unknown").contains("available library"));
    let send = app.library_editors.files.stand_in();
    assert!(export(&mut app, "example-parts").contains("Finish the current library file operation"));

    app.run_window_command(&egui::Context::default(), "app.library_browse", &json!({"view": view, "source": "example-parts"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    click(&mut h, "Export library as a bundle file");
    assert!(h.state().message.as_deref().is_some_and(|message| message.contains("Finish the current library file operation")));

    send.send(Ok(Outcome::Exported("Exported Native Parts Demo (4 items).".into()))).unwrap();
    h.state_mut().library_editors.poll();
    assert_eq!(h.state().library_editors.notice(), Some("Exported Native Parts Demo (4 items)."));
}
