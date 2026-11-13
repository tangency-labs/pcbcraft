use super::*;
use crate::Dialog;
use crate::tests::click;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pcbcraft_engine::library::sources::Kind;
use pcbcraft_engine::{
    Session,
    persistence::{decode_documents, encode_documents},
};

/// Original native symbol data uses a distinct manifest identity and human-readable search name.
fn documents() -> sources::Documents {
    let mut symbol: serde_json::Value =
        serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../libraries/starter/symbols/resistor-v2.json"))).unwrap();
    symbol["name"] = json!("Workshop resistor");
    sources::Documents::from([
        ("library.json".into(), json!({"format":"pcbcraft-library/1","id":"workshop","name":"Workshop","license":"MIT OR Apache-2.0"}).to_string()),
        ("symbols/resistor.json".into(), symbol.to_string()),
    ])
}

/// Source management opens the workspace and removing a registration preserves pinned data.
#[test]
fn source_workspace_preserves_project_and_uses_real_catalog() {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({}));
    app.session.execute(sources::LOAD, &json!({"source":"workshop-source","documents":documents()})).unwrap();
    let before = app.session.inspect();
    let snapshot = app.session.snapshot().unwrap();
    app.open_library_workspace(&egui::Context::default());
    let mut h = Harness::builder().with_size(egui::vec2(1600., 1000.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    click(&mut h, "Workshop");
    h.get_by_label("Symbol workshop/resistor r2");
    assert_eq!(h.state().workspace_key(), "library");
    assert_eq!(h.state().session.inspect(), before);
    let app = h.state_mut();
    app.library_sources.remove("workshop-source", &mut app.session);
    assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
    assert_eq!(
        encode_documents(snapshot.clone()).unwrap(),
        encode_documents(decode_documents(&encode_documents(snapshot).unwrap()).unwrap()).unwrap()
    );
}

/// Removing a source during a blocked read retains the worker slot and discards its late result.
/// A disconnected refresh removes stale catalog data but preserves the authored project snapshot.
#[test]
fn removed_and_disconnected_loads_cannot_restore_stale_catalog_entries() {
    let source = Arc::new(NativeSource::load(&documents()).unwrap());
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let before = session.snapshot().unwrap();
    let mut manager = Sources::default();
    let (send, receive) = mpsc::channel();
    manager.statuses.insert("loading".into(), "Loading…".into());
    manager.job =
        Some(Job { installing: false, source: "loading".into(), previous_status: None, receive, generation: session.libraries().generation() });
    manager.remove("loading", &mut session);
    assert!(manager.job.is_some());
    send.send(Ok(Some((source.clone(), None, false)))).unwrap();
    manager.poll(&mut session, &egui::Context::default());
    assert!(manager.job.is_none());
    assert!(session.libraries().search("Workshop", None).is_empty());
    session.install_library_source("loading", source).unwrap();
    let (send, receive) = mpsc::channel();
    drop(send);
    manager.statuses.insert("loading".into(), "Loading…".into());
    manager.job =
        Some(Job { installing: false, source: "loading".into(), previous_status: None, receive, generation: session.libraries().generation() });
    manager.poll(&mut session, &egui::Context::default());
    assert!(session.libraries().search("Workshop", None).is_empty());
    assert!(manager.statuses["loading"].contains("disconnected"));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// A delayed filesystem result cannot overwrite a newer catalog change made through MCP.
#[test]
fn newer_source_command_invalidates_an_inflight_folder_result() {
    let mut session = Session::default();
    let mut manager = Sources::default();
    let (send, receive) = mpsc::channel();
    manager.statuses.insert("loading".into(), "Loading…".into());
    manager.job =
        Some(Job { installing: false, source: "loading".into(), previous_status: None, generation: session.libraries().generation(), receive });
    session.execute(sources::LOAD, &json!({"source":"mcp","documents":documents()})).unwrap();
    send.send(Ok(Some((Arc::new(NativeSource::load(&documents()).unwrap()), None, false)))).unwrap();
    manager.poll(&mut session, &egui::Context::default());
    assert!(manager.statuses["loading"].contains("changed during loading"));
    assert_eq!(session.libraries().inspect().as_array().unwrap().len(), 3);
    assert_eq!(session.libraries().search("Workshop", None)[0].0, "mcp");
}

/// Cancelling Add/Relocate restores the prior diagnostic without changing availability,
/// personal locators, project snapshots or history, including after a failed refresh.
#[test]
fn cancelled_picker_restores_the_previous_source_status() {
    for previous in [None, Some("Available"), Some("Library folder is missing")] {
        let mut session = Session::default();
        session.apply("project.new", &json!({})).unwrap();
        let mut manager = Sources::default();
        if let Some(status) = previous {
            session.install_library_source("folder", Arc::new(NativeSource::load(&documents()).unwrap())).unwrap();
            manager.registered.push(Registration { source: "folder".into(), path: "/missing/workshop".into(), name: String::new(), writable: true });
            manager.statuses.insert("folder".into(), status.into());
            if status != "Available" {
                session.mark_library_unavailable("folder");
            }
        }
        let before = session.snapshot().unwrap();
        let revision = session.revision();
        let catalog = session.libraries().inspect();
        let generation = session.libraries().generation();
        let registrations = manager.storage_text();
        let (send, receive) = mpsc::channel();
        let previous_status = manager.statuses.insert("folder".into(), "Loading…".into());
        manager.job = Some(Job { installing: false, source: "folder".into(), previous_status, receive, generation });
        send.send(Ok(None)).unwrap();
        manager.poll(&mut session, &egui::Context::default());
        assert!(manager.job.is_none());
        assert_eq!(manager.statuses.get("folder").map(String::as_str), previous);
        assert_eq!(manager.storage_text(), registrations);
        assert_eq!(session.libraries().inspect(), catalog);
        assert_eq!(session.libraries().generation(), generation);
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
}

/// The live editor adapter loads/searches the same catalog that Parts reads, without changing
/// the open project's snapshot, revision, dirty state or undo availability.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_source_commands_share_parts_catalog_without_project_edits() {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({}));
    let before = app.session.inspect();
    let dirty = app.files.dirty(&app.session);
    let ctx = egui::Context::default();
    let request = json!({"method":"engine.execute","params":{"command":sources::LOAD,"params":{"source":"live","documents":documents()}}});
    pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: &mut app, ctx: &ctx }, &request).unwrap();
    let request = json!({"method":"engine.execute","params":{"command":sources::SEARCH,"params":{"query":"Workshop","kind":"symbol"}}});
    let found = pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: &mut app, ctx: &ctx }, &request).unwrap();
    assert_eq!(found[0]["source"], "live");
    assert_eq!(app.session.libraries().search("Workshop", Some(Kind::Symbol)).len(), 1);
    assert_eq!(app.session.inspect(), before);
    assert_eq!(app.files.dirty(&app.session), dirty);
}

/// Restored registrations perform bounded reads off-thread. Relocating a folder preserves item
/// identity, and a failed refresh leaves the source unavailable instead of offering stale content.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn personal_folder_registration_restores_and_reports_missing_sources() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("symbols")).unwrap();
    for (path, text) in documents() {
        std::fs::write(root.path().join(path), text).unwrap();
    }
    let registered = vec![Registration { source: "folder".into(), path: root.path().to_str().unwrap().into(), name: String::new(), writable: true }];
    let mut manager = Sources::restore(Some(serde_json::to_string(&registered).unwrap()));
    let mut session = Session::default();
    let pump = |manager: &mut Sources, session: &mut Session| {
        let start = std::time::Instant::now();
        loop {
            manager.poll(session, &egui::Context::default());
            if manager.job.is_none() && manager.pending.is_empty() {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    pump(&mut manager, &mut session);
    assert_eq!(session.libraries().search("Workshop", Some(Kind::Symbol)).len(), 1);
    let text = manager.storage_text();
    assert!(text.contains("folder"));
    assert!(!text.contains("Workshop resistor"));
    let missing = root.path().join("missing").to_str().unwrap().to_owned();
    manager.load_path("folder".into(), missing, &egui::Context::default(), session.libraries().generation()).unwrap();
    pump(&mut manager, &mut session);
    assert!(session.libraries().search("Workshop", None).is_empty());
    assert_ne!(manager.statuses["folder"], "Available");
}

/// Load an actual desktop folder worker and drive the app until its bounded result is adopted.
fn load_desktop_source(h: &mut Harness<'_, PcbApp>, path: &std::path::Path) {
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    app.library_sources.load_path("desktop-parts".into(), path.to_str().unwrap().into(), &ctx, app.session.libraries().generation()).unwrap();
    let start = std::time::Instant::now();
    loop {
        h.run();
        if h.state().library_sources.job.is_none() {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Desktop folder relocation, either package workflow and a source-free handoff use real GUI/worker/storage paths.
#[test]
fn desktop_native_part_relocation_and_offline_handoff() {
    use pcbcraft_engine::persistence::{ENTRY_POINT, encode_packed};
    for package_before_placement in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original-library");
        let files = [
            ("library.json", include_str!("../../../../examples/native-parts/library.json")),
            ("symbols/resistor.json", include_str!("../../../../examples/native-parts/symbols/resistor.json")),
            ("footprints/resistor-0603.json", include_str!("../../../../examples/native-parts/footprints/resistor-0603.json")),
            ("categories/resistor.json", include_str!("../../../../examples/native-parts/categories/resistor.json")),
            ("parts/resistor.json", include_str!("../../../../examples/native-parts/parts/resistor.json")),
            ("manufacturer-parts/test.json", include_str!("../../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json")),
        ];
        for (path, text) in files {
            let path = original.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let mut app = PcbApp::default();
        app.parts.set_repeat(false);
        app.run("project.new", &json!({"create_board":true}));
        let mut h =
            Harness::builder().with_size(egui::vec2(1400., 1000.)).with_max_steps(40).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        load_desktop_source(&mut h, &original);
        h.key_press(egui::Key::A);
        h.run();
        click(&mut h, "Native Parts Demo");
        click(&mut h, "Generic resistor");
        if package_before_placement {
            h.get_by_role_and_label(egui::Role::ComboBox, "Manufacturer part").click();
            h.run();
            click(&mut h, "Illustrative Manufacturer · TEST-R100M-0603");
        } else {
            h.get_by_role_and_label(egui::Role::ComboBox, "Package").click();
            h.run();
            click(&mut h, "Unassigned");
        }
        click(&mut h, "Place");
        click(&mut h, "Schematic canvas");
        let component = h.state().session.project().unwrap().components().next().unwrap().id();
        if !package_before_placement {
            assert!(h.state().session.project().unwrap().footprint_assignments().is_empty());
            h.state_mut().begin_native_part_properties(component.clone());
            h.run();
            h.get_by_role_and_label(egui::Role::ComboBox, "Package").click();
            h.run();
            click(&mut h, "0603");
            click(&mut h, "Apply");
            assert!(matches!(h.state().dialog, Dialog::None));
        }
        let snapshot = h.state().session.snapshot().unwrap();
        let board = snapshot.board_target().unwrap();
        let docs = encode_documents(snapshot.clone()).unwrap();
        let folder = root.path().join("handoff");
        pcbcraft_storage::save(&folder, None, &docs, &mut |_| Ok(())).unwrap();
        let archive = root.path().join("handoff.pc");
        std::fs::write(&archive, encode_packed(snapshot.clone()).unwrap()).unwrap();
        let moved = root.path().join("relocated-library");
        std::fs::rename(&original, &moved).unwrap();
        load_desktop_source(&mut h, &original);
        assert!(h.state().session.libraries().search("", Some(Kind::Part)).is_empty());
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
        load_desktop_source(&mut h, &moved);
        assert_eq!(h.state().session.libraries().search("", Some(Kind::Part)).len(), 1);
        // Compare the parsed path: the JSON text escapes the backslashes of a Windows path.
        let registered: serde_json::Value = serde_json::from_str(&h.state().library_sources.storage_text()).unwrap();
        assert!(
            registered.as_array().unwrap().iter().any(|registration| registration["path"] == moved.to_str().unwrap()),
            "the relocated folder is registered: {registered}"
        );
        let app = h.state_mut();
        app.library_sources.remove("desktop-parts", &mut app.session);
        std::fs::remove_dir_all(&moved).unwrap();
        for path in [&folder, &folder.join(ENTRY_POINT), &archive] {
            let reopened = pcbcraft_storage::open(path).unwrap();
            assert_eq!(reopened.board_target(), Some(board));
            assert_eq!(encode_documents(reopened.clone()).unwrap(), docs);
            let mut clean = Session::default();
            assert!(clean.libraries().search("", Some(Kind::Part)).is_empty());
            clean.open_snapshot(reopened);
            let selected = clean.project().unwrap().components().next().unwrap().part.as_ref().unwrap();
            assert_eq!(selected.package.as_deref(), Some("0603"));
            assert_eq!(selected.manufacturer.is_some(), package_before_placement);
            clean
                .apply(
                    pcbcraft_engine::PLACE_ASSIGNED_FOOTPRINT,
                    &json!({"component":component,"board":board.board,"x_nm":0,"y_nm":0,"expected_revision":clean.revision()}),
                )
                .unwrap();
            let placed = encode_documents(clean.snapshot().unwrap()).unwrap();
            assert_eq!(encode_documents(decode_documents(&placed).unwrap()).unwrap(), placed);
        }
    }
}
