use super::*;
use std::time::{Duration, Instant};

/// Original connector/header definitions exercise the complete host folder and catalog path.
#[allow(clippy::unwrap_used)]
fn fixture() -> (PcbApp, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workshop");
    let manifest = engine::new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
    pcbcraft_storage::library::publishing::create(&root, &manifest).unwrap();
    let symbol = Item::edit_symbol(pcbcraft_engine::library::starter().symbol("connector").unwrap()).unwrap();
    let footprint: pcbcraft_engine::board::definition::FootprintDefinition =
        serde_json::from_slice(include_bytes!("../../../../../libraries/starter/footprints/header-1x02-254.json")).unwrap();
    for mut item in [symbol, Item::edit_footprint(&footprint).unwrap()] {
        item.library = "workshop".into();
        let prepared = engine::prepare(Arc::new(item), &manifest, 0, None, false).unwrap();
        pcbcraft_storage::library::publishing::publish(&root, &prepared).unwrap();
    }
    let mut app = PcbApp::default();
    app.session.apply("project.new", &json!({})).unwrap();
    app.session.install_library_source("workshop-source", Arc::new(pcbcraft_storage::library::load(&root).unwrap())).unwrap();
    app.library_sources.registered.push(crate::native_sources::Registration {
        source: "workshop-source".into(),
        path: root.to_str().unwrap().into(),
        name: "Workshop".into(),
        writable: true,
    });
    app.place_library_workspace(&Context::default(), pcbcraft_chrome::workspace::Placement::Here).unwrap();
    (app, temp)
}

/// Opening an older revision warns before writing, while Edit copy gets a separate identity.
#[test]
fn older_catalog_edit_requires_consent_and_copy_is_explicit() {
    let (mut app, _temp) = fixture();
    let ctx = Context::default();
    let view = app.views.active.id();
    app.run_window_command(&ctx, "app.library_browse", &json!({"view":view.0,"source":"workshop-source","type":1,"item":"connector"})).unwrap();
    app.edit_catalog_selection(&ctx, false).unwrap();
    let id = app.library_editors.active.unwrap();
    app.library_editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
    let path = app.library_sources.registered[0].path.clone();
    let manifest = pcbcraft_storage::library::publishing::read_manifest(std::path::Path::new(&path)).unwrap();
    let prepared = engine::prepare(app.library_editors.documents[&id].session.snapshot(), &manifest, 2, Some(2), false).unwrap();
    pcbcraft_storage::library::publishing::publish(std::path::Path::new(&path), &prepared).unwrap();
    app.session.install_library_source("workshop-source", Arc::new(pcbcraft_storage::library::load(std::path::Path::new(&path)).unwrap())).unwrap();
    let before = app.library_editors.documents[&id].session.snapshot();
    assert!(app.publish_library_item(&ctx, id, 1, "workshop-source", false).unwrap_err().contains("intervening changes"));
    assert!(Arc::ptr_eq(&before, &app.library_editors.documents[&id].session.snapshot()));
    app.publish_library_item(&ctx, id, 1, "workshop-source", true).unwrap();
    finish(&mut app, &ctx);
    assert_eq!(app.library_editors.documents[&id].session.published().unwrap().revision, 4);
    app.copy_library_document(&ctx, id).unwrap();
    let copy = app.library_editors.active.unwrap();
    assert_ne!(copy, id);
    assert_ne!(app.library_editors.documents[&copy].session.snapshot().id, before.id);
    assert_eq!(app.library_editors.documents[&copy].session.snapshot().library, "user");
    assert!(app.library_editors.documents[&copy].session.published().is_none());
}

/// Wait only for the bounded test worker; a stuck or disconnected writer fails rather than hangs.
fn finish(app: &mut PcbApp, ctx: &Context) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.library_editors.publisher.busy() && Instant::now() < deadline {
        app.poll_library_publication(ctx);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!app.library_editors.publisher.busy(), "publisher did not complete");
}

/// Live edits remain enabled during background publication, and later geometry stays unsaved.
#[test]
fn live_edit_during_publication_retains_later_draft_and_excludes_source_writes() {
    use pcbcraft_mcp::live::Editor;
    let (mut app, _temp) = fixture();
    let ctx = Context::default();
    app.new_library_item(Kind::Symbol);
    let id = app.library_editors.active.unwrap();
    let item = app.library_editors.documents[&id].session.snapshot();
    let inspected = crate::window_commands::inspect_editor(&app);
    assert_eq!(inspected["library_destinations"][0]["source"], "workshop-source");
    assert_eq!(inspected["library_destinations"][0]["library"], "workshop");
    assert!(inspected["library_destinations"][0].get("path").is_none());
    app.publish_library_item(&ctx, id, 0, "workshop-source", false).unwrap();
    let mut live = crate::live_control::LiveEditor { app: &mut app, ctx: &ctx };
    assert!(live.refusal("app.library_edit").is_none());
    assert!(live.refusal(sources::LOAD).is_some());
    assert!(live.refusal(sources::REMOVE).is_some());
    let action = json!({"operation":"metadata","name":"Later symbol","license":item.license,"source":item.source,"reference_prefix":item.reference_prefix,"default_value":item.default_value});
    pcbcraft_mcp::live::call_editor(
        &mut live,
        &json!({"method":"engine.execute","params":{"command":"app.library_edit","params":{"document":id,"expected_revision":0,"action":action}}}),
    )
    .unwrap();
    finish(&mut app, &ctx);
    let session = &app.library_editors.documents[&id].session;
    assert!(session.dirty());
    assert!(session.publication_dirty());
    assert_eq!(session.snapshot().name, "Later symbol");
    assert_eq!(session.snapshot().library, "workshop");
    assert_eq!(session.published().unwrap().revision, 1);
    assert_eq!(app.session.libraries().symbol("workshop", &item.id, 1).unwrap().name, "Custom symbol");
    app.publish_library_item(&ctx, id, 1, "workshop-source", false).unwrap();
    finish(&mut app, &ctx);
    assert!(!app.library_editors.documents[&id].session.dirty());
    assert_eq!(app.session.libraries().symbol("workshop", &item.id, 2).unwrap().name, "Later symbol");
}

/// Choosing another collection is Save As: it publishes current edits without changing the
/// original library, then keeps the editor on its new address for subsequent ordinary saves.
#[test]
fn publish_to_different_library_and_new_item_in_same_library_are_explicit_copies() {
    for (kind, item_id, original_revision) in [(1, "connector", 2), (2, "header-1x02-254", 3)] {
        let (mut app, temp) = fixture();
        let ctx = Context::default();
        let view = app.views.active.id();
        app.run_window_command(&ctx, "app.library_browse", &json!({"view":view.0,"source":"workshop-source","type":kind,"item":item_id})).unwrap();
        let before = app.session.libraries().source_items("workshop-source").iter().find(|item| item.key.id == item_id).unwrap().inspect();
        app.edit_catalog_selection(&ctx, false).unwrap();
        let id = app.library_editors.active.unwrap();
        app.library_editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
        let destination = temp.path().join("team");
        let manifest = engine::new_library("team", "Team library", "MIT OR Apache-2.0").unwrap();
        let parsed = pcbcraft_storage::library::publishing::create(&destination, &manifest).unwrap();
        app.adopt_published_source("team-source", destination.to_str().unwrap(), app.session.libraries().generation(), Ok(Arc::new(parsed))).unwrap();
        app.publish_library_item(&ctx, id, 1, "team-source", false).unwrap();
        finish(&mut app, &ctx);
        let session = &app.library_editors.documents[&id].session;
        let copied = session.snapshot();
        assert_eq!(copied.library, "team");
        assert_ne!(copied.id, item_id);
        assert!(!session.dirty());
        let original = app.session.libraries().source_items("workshop-source").iter().find(|item| item.key.id == item_id).unwrap();
        assert_eq!(original.inspect(), before);
        assert_eq!(original.key.revision, original_revision);
        let revision = session.revision();
        app.publish_library_item_as(&ctx, id, revision, "team-source", false, true).unwrap();
        finish(&mut app, &ctx);
        let session = &app.library_editors.documents[&id].session;
        assert_ne!(session.snapshot().id, copied.id);
        assert_eq!(session.snapshot().library, "team");
        assert!(app.session.libraries().source_items("team-source").iter().any(|item| item.key.id == copied.id));
    }
}

/// New library creation returns to the same Publish dialog with that collection selected.
#[test]
fn create_collection_from_publish_resumes_original_document_without_binding_early() {
    let (mut app, temp) = fixture();
    let ctx = Context::default();
    app.new_library_item(Kind::Symbol);
    let id = app.library_editors.active.unwrap();
    app.begin_library_publish(id).unwrap();
    let Dialog::LibraryPublish(draft) = std::mem::replace(&mut app.dialog, Dialog::None) else { panic!("publish dialog missing") };
    let root = temp.path().join("new-collection");
    let manifest = engine::new_library("new-collection", "New collection", "MIT OR Apache-2.0").unwrap();
    let parsed = pcbcraft_storage::library::publishing::create(&root, &manifest).unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    app.library_editors.publisher.pending = Some(receive);
    send.send(Event::Finished(Ok(Completion::Created {
        source: "new-collection-source".into(),
        path: root.to_str().unwrap().into(),
        parsed: Arc::new(parsed),
        generation: app.session.libraries().generation(),
        return_to: Some(Box::new(draft)),
    })))
    .unwrap();
    app.poll_library_publication(&ctx);
    let Dialog::LibraryPublish(draft) = &app.dialog else { panic!("publish dialog was not resumed") };
    assert_eq!(draft.document, id);
    assert_eq!(draft.source, "new-collection-source");
    assert!(app.library_editors.documents[&id].session.published().is_none());
    assert_eq!(app.library_publication_status(id).unwrap().0, "Publish to: New collection");
}

/// A completed collection picker waits for an existing close decision before resuming Publish.
#[test]
fn creation_resume_does_not_replace_another_dialog() {
    let (mut app, _temp) = fixture();
    let ctx = Context::default();
    app.new_library_item(Kind::Symbol);
    let id = app.library_editors.active.unwrap();
    app.begin_library_publish(id).unwrap();
    let Dialog::LibraryPublish(draft) = std::mem::replace(&mut app.dialog, Dialog::LibraryClose(Some(id))) else { panic!("missing publish dialog") };
    let (send, receive) = mpsc::sync_channel(1);
    app.library_editors.publisher.pending = Some(receive);
    send.send(Event::Finished(Ok(Completion::Cancelled(Some(Box::new(draft)))))).unwrap();
    app.poll_library_publication(&ctx);
    assert!(matches!(app.dialog,Dialog::LibraryClose(Some(owner)) if owner==id));
    assert!(app.library_editors.publisher.resume.is_some());
    app.dialog = Dialog::None;
    app.poll_library_publication(&ctx);
    assert!(matches!(app.dialog,Dialog::LibraryPublish(draft) if draft.document==id));
}

/// Real GUI and MCP commands retain connector/header identities across publish and fresh reopen.
#[test]
fn live_catalog_edit_publish_restart_and_reopen_preserves_item_and_project() {
    for (kind, item_id) in [(1, "connector"), (2, "header-1x02-254")] {
        let (mut app, _temp) = fixture();
        let ctx = Context::default();
        let view = app.views.active.id();
        let before = app.session.snapshot().unwrap();
        app.run_window_command(&ctx, "app.library_browse", &json!({"view":view.0,"source":"workshop-source","type":kind,"item":item_id})).unwrap();
        app.run_window_command(&ctx, "app.library_catalog_edit", &json!({"view":view.0})).unwrap();
        let id = app.library_editors.active.unwrap();
        assert!(!app.library_editors.documents[&id].session.dirty());
        let original = app.library_editors.documents[&id].session.snapshot();
        app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":0,"action":{"operation":"delete","objects":[3]}}))
            .unwrap();
        // Saving an editable draft is not publication and must not suppress the Publish action.
        let current = app.library_editors.documents[&id].session.snapshot();
        app.library_editors.documents.get_mut(&id).unwrap().session.mark_saved(current);
        assert!(app.library_editors.documents[&id].session.publication_dirty());
        app.run_window_command(&ctx, "app.library_publish", &json!({"document":id,"expected_revision":1,"source":"workshop-source"})).unwrap();
        let another = app.library_editors.open(Item::new(Kind::Symbol).unwrap(), false).unwrap();
        finish(&mut app, &ctx);
        assert_eq!(app.library_editors.active, Some(another));
        let session = &app.library_editors.documents[&id].session;
        assert!(!session.dirty(), "{:?}", app.library_editors.notice);
        assert_eq!(session.published().unwrap().id, original.id);
        let published_revision = session.published().unwrap().revision;
        assert!(published_revision > original.revision);
        assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
        let path = app.library_sources.registered[0].path.clone();
        let source = pcbcraft_storage::library::load(std::path::Path::new(&path)).unwrap();
        let mut restarted = PcbApp::default();
        restarted.session.install_library_source("workshop-source", Arc::new(source)).unwrap();
        restarted.library_sources.registered = app.library_sources.registered.clone();
        restarted.place_library_workspace(&ctx, pcbcraft_chrome::workspace::Placement::Here).unwrap();
        let view = restarted.views.active.id();
        restarted
            .run_window_command(&ctx, "app.library_browse", &json!({"view":view.0,"source":"workshop-source","type":kind,"item":item_id}))
            .unwrap();
        restarted.run_window_command(&ctx, "app.library_catalog_edit", &json!({"view":view.0})).unwrap();
        let reopened = &restarted.library_editors.documents[&restarted.library_editors.active.unwrap()].session;
        assert_eq!(reopened.published().unwrap().revision, published_revision);
        assert_eq!(reopened.snapshot().id, original.id);
        assert!(
            restarted
                .session
                .libraries()
                .source_items("workshop-source")
                .iter()
                .any(|item| item.key.id == item_id && item.key.revision == original.revision)
        );
    }
}

/// A failed host write retains the original published address, undo history and changed draft.
#[test]
fn failed_publication_keeps_draft_and_stale_requests_cannot_publish() {
    let (mut app, temp) = fixture();
    let ctx = Context::default();
    let view = app.views.active.id();
    app.run_window_command(&ctx, "app.library_browse", &json!({"view":view.0,"source":"workshop-source","type":1,"item":"connector"})).unwrap();
    app.edit_catalog_selection(&ctx, false).unwrap();
    let id = app.library_editors.active.unwrap();
    app.library_editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
    let before = app.library_editors.documents[&id].session.snapshot();
    assert!(app.publish_library_item(&ctx, id, 0, "workshop-source", false).is_err());
    std::fs::rename(temp.path().join("workshop"), temp.path().join("moved")).unwrap();
    app.publish_library_item(&ctx, id, 1, "workshop-source", false).unwrap();
    finish(&mut app, &ctx);
    let session = &app.library_editors.documents[&id].session;
    assert!(Arc::ptr_eq(&before, &session.snapshot()));
    assert!(session.dirty());
    assert!(session.can_undo());
    assert_eq!(session.published().unwrap().revision, 2);
    assert_eq!(pcbcraft_storage::library::load(&temp.path().join("moved")).unwrap().items().len(), 2);
}

/// Publication still saves its owner when only the catalog refresh fails; retry never duplicates it.
#[test]
fn successful_write_with_failed_refresh_is_saved_and_not_republished() {
    let (mut app, _temp) = fixture();
    let ctx = Context::default();
    let id = app.library_editors.open(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    let item = app.library_editors.documents[&id].session.snapshot();
    let path = app.library_sources.registered[0].path.clone();
    let manifest = pcbcraft_storage::library::publishing::read_manifest(std::path::Path::new(&path)).unwrap();
    let prepared = engine::prepare(item, &manifest, 0, None, false).unwrap();
    pcbcraft_storage::library::publishing::publish(std::path::Path::new(&path), &prepared).unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    app.library_editors.publisher.pending = Some(receive);
    send.send(Event::Finished(Ok(Completion::Published(
        Request { document: id, source: "workshop-source".into(), path, generation: app.session.libraries().generation(), prepared },
        Err("injected refresh failure".into()),
    ))))
    .unwrap();
    app.poll_library_publication(&ctx);
    assert!(!app.library_editors.documents[&id].session.dirty());
    assert!(app.library_editors.notice.as_ref().unwrap().contains("Refresh failed"));
    assert!(app.publish_library_item(&ctx, id, 0, "workshop-source", false).unwrap_err().contains("already published"));
}

/// A disconnected writer retains exact retry bytes; verification adopts the existing record once.
#[test]
fn disconnected_worker_recovery_does_not_create_a_second_revision() {
    let (mut app, _temp) = fixture();
    let ctx = Context::default();
    let id = app.library_editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let draft = app.library_editors.documents[&id].session.snapshot();
    app.library_editors.documents.get_mut(&id).unwrap().session.mark_saved(draft);
    let path = app.library_sources.registered[0].path.clone();
    let manifest = pcbcraft_storage::library::publishing::read_manifest(std::path::Path::new(&path)).unwrap();
    let prepared = engine::prepare(app.library_editors.documents[&id].session.snapshot(), &manifest, 0, None, false).unwrap();
    pcbcraft_storage::library::publishing::publish(std::path::Path::new(&path), &prepared).unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    app.library_editors.publisher.pending = Some(receive);
    send.send(Event::Prepared(Request {
        document: id,
        source: "workshop-source".into(),
        path: path.clone(),
        generation: app.session.libraries().generation(),
        prepared,
    }))
    .unwrap();
    app.poll_library_publication(&ctx);
    drop(send);
    app.poll_library_publication(&ctx);
    assert!(app.library_editors.publisher.recovery.is_some());
    assert!(!app.library_editors.documents[&id].session.dirty());
    assert!(app.library_editors.document_unsaved(id));
    assert!(app.library_editors.close(id, false).is_err());
    app.publish_library_item(&ctx, id, 0, "workshop-source", false).unwrap();
    finish(&mut app, &ctx);
    assert!(!app.library_editors.documents[&id].session.dirty());
    assert_eq!(pcbcraft_storage::library::load(std::path::Path::new(&path)).unwrap().items().len(), 3);
}

/// The Parts Library ribbon opens real authoring actions through shared controls in both themes.
#[test]
fn library_ribbon_new_item_and_publish_dialog_use_existing_workspace_host() {
    use egui_kittest::{Harness, kittest::Queryable};
    for theme in [crate::ThemeChoice::Dark, crate::ThemeChoice::Light] {
        let (mut app, _temp) = fixture();
        app.preferences.theme = theme;
        let view = app.views.active.id();
        app.run_window_command(
            &Context::default(),
            "app.library_browse",
            &json!({"view":view.0,"source":"workshop-source","type":1,"item":"connector"}),
        )
        .unwrap();
        let mut h =
            Harness::builder().with_size(egui::vec2(1500., 900.)).with_max_steps(64).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "New symbol");
        assert_eq!(h.state().library_editors.active_kind(), Some(Kind::Symbol));
        let id = h.state().library_editors.active.unwrap();
        assert_eq!(h.state().library_editors.documents[&id].publish_source.as_deref(), Some("workshop-source"));
        assert_eq!(h.state().library_publication_status(id).unwrap().0, "Publish to: Workshop");
        h.state_mut().begin_library_publish(id).unwrap();
        h.run();
        assert!(h.query_all_by_label("Publish Custom symbol").next().is_some());
        assert!(h.query_all_by_label("Publish").next().is_some());
        assert!(h.query_all_by_label("Item ID").next().is_none());
        assert!(h.query_all_by_label("Folder").next().is_none());
        crate::tests::click(&mut h, "Expand Details");
        assert!(h.query_all_by_label("Item ID").next().is_some());
        assert!(h.query_all_by_label("Folder").next().is_some());
        crate::tests::click(&mut h, "Collapse Details");
        crate::tests::click(&mut h, "Workshop");
        crate::tests::click(&mut h, "New library…");
        assert!(matches!(h.state().dialog, Dialog::NewLibrary(_)));
        crate::tests::click(&mut h, "Cancel");
        assert!(matches!(h.state().dialog, Dialog::LibraryPublish(_)));
        crate::tests::click(&mut h, "Cancel");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(h.state().library_editors.documents[&id].session.published().is_none());
    }
}
