use super::*;
use pcbcraft_engine::library::parts::Package;
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Use original bundled definitions and a real unused native publication folder.
pub(super) fn fixture() -> (PcbApp, tempfile::TempDir, u64) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workshop");
    let manifest = pcbcraft_engine::authoring::publishing::new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
    pcbcraft_storage::library::publishing::create(&root, &manifest).unwrap();
    let mut app = PcbApp::default();
    app.session.apply("project.new", &json!({})).unwrap();
    app.session.install_library_source("workshop", Arc::new(pcbcraft_storage::library::load(&root).unwrap())).unwrap();
    app.library_sources.registered.push(crate::native_sources::Registration {
        source: "workshop".into(),
        path: root.to_str().unwrap().into(),
        name: "Workshop".into(),
        writable: true,
    });
    let id = app
        .new_part_document(&egui::Context::default(), Some(Reference { library: "basic-components".into(), id: "connector".into(), revision: 1 }))
        .unwrap();
    let mut record = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(p) = &mut record else { panic!() };
    p.name = "Original connector".into();
    p.packages.insert(
        "header".into(),
        Package {
            name: "Header".into(),
            footprint: Reference { library: "basic-components".into(), id: "header-1x02-254".into(), revision: 3 },
            mapping: BTreeMap::from([("1".into(), vec!["1".into()]), ("2".into(), vec!["2".into()])]),
            unconnected_pads: Vec::new(),
        },
    );
    app.replace_part_document(id, 1, record).unwrap();
    (app, temp, id)
}
/// Bounded test waits fail if a native publication slot does not release.
fn finish(app: &mut PcbApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.library_editors.publisher.busy() && Instant::now() < deadline {
        app.poll_library_publication(ctx);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!app.library_editors.publisher.busy());
}
/// The actual shared host path publishes native generic/orderable records without project edits.
#[test]
fn native_part_and_orderable_publication_reopen_and_stale_edits() {
    let (mut app, _temp, id) = fixture();
    let ctx = egui::Context::default();
    let before = app.session.inspect();
    let revision = app.library_editors.parts[&id].session.revision();
    app.publish_library_item(&ctx, id, revision, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    let doc = &app.library_editors.parts[&id];
    assert!(!doc.dirty(), "{:?}", app.library_editors.notice());
    let reference = doc.session.record().reference();
    app.session.libraries().resolve_part(&reference).unwrap();
    assert_eq!(app.session.inspect(), before);
    let id = app.new_orderable_document(&ctx, reference).unwrap();
    let mut record = app.library_editors.parts[&id].session.record().clone();
    let Record::Orderable(m) = &mut record else { panic!() };
    m.manufacturer = "Synthetic test maker".into();
    m.mpn = "TEST-2PIN".into();
    app.replace_part_document(id, 1, record.clone()).unwrap();
    assert!(app.replace_part_document(id, 1, record).is_err());
    app.publish_library_item(&ctx, id, 2, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    assert!(!app.library_editors.parts[&id].dirty());
    assert_eq!(app.session.inspect(), before);
    let source = pcbcraft_storage::library::load(std::path::Path::new(&app.library_sources.registered[0].path)).unwrap();
    assert_eq!(source.items().len(), 2);
    assert!(source.problems().is_empty());
}
/// Invalid mappings and unapplied input protect prior native bytes, history and close decisions.
#[test]
fn invalid_mapping_and_pending_input_never_replace_published_records() {
    let (mut app, temp, id) = fixture();
    let ctx = egui::Context::default();
    app.publish_library_item(&ctx, id, 2, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    let folder = temp.path().join("workshop/parts");
    let file = std::fs::read_dir(&folder).unwrap().next().unwrap().unwrap().path();
    let bytes = std::fs::read(&file).unwrap();
    let mut record = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(p) = &mut record else { panic!() };
    p.packages.get_mut("header").unwrap().mapping.remove("2");
    let revision = app.library_editors.parts[&id].session.revision();
    app.replace_part_document(id, revision, record).unwrap();
    let revision = app.library_editors.parts[&id].session.revision();
    assert!(app.publish_library_item(&ctx, id, revision, "workshop", false).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 1);
    assert!(app.library_editors.close_part(id, false).is_err());
    app.begin_project_close(&ctx);
    assert!(matches!(app.dialog, Dialog::LibraryClose(None)));
    app.library_editors.parts.get_mut(&id).unwrap().pending.insert("test:field".into(), "bad".into());
    assert!(app.begin_library_publish(id).is_err());
    app.library_editors.close_part(id, true).unwrap();
    assert!(!app.library_editors.parts.contains_key(&id));
}
/// Real workspace input creates a document and renders it in wide dark and compact light layouts.
#[test]
fn new_generic_part_uses_existing_workspace_and_discard_dialog() {
    use egui_kittest::{Harness, kittest::Queryable};
    for (width, light) in [(1800., false), (620., true)] {
        let mut app = PcbApp::default();
        if light {
            app.preferences.theme = crate::preferences::ThemeChoice::Light;
        }
        app.place_library_workspace(&egui::Context::default(), pcbcraft_chrome::workspace::Placement::Here).unwrap();
        let mut h =
            Harness::builder().with_size(egui::vec2(width, 1000.)).with_max_steps(64).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "New generic part");
        assert_eq!(h.state().library_editors.parts.len(), 1, "width={width} message={:?}", h.state().message);
        assert_eq!(h.state().workspace_key(), "library");
        if width < 1050. {
            h.get_by_label("Items").click();
            h.run();
        }
        let id = *h.state().library_editors.parts.keys().next().unwrap();
        assert_eq!(h.state().views.active.library.as_ref().unwrap().part_document, Some(id));
        h.get_by_label("Close document").click();
        h.run();
        assert!(matches!(h.state().dialog,Dialog::LibraryClose(Some(owner)) if owner==id));
    }
}

/// A visible but unconfirmed generic part is recovered under the same immutable address;
/// edits made while I/O owns the capture stay unpublished after confirmation.
#[test]
fn uncertain_part_write_reuses_exact_capture_and_preserves_later_edits() {
    use crate::library_editors::publishing::{Capture, Request};
    use pcbcraft_engine::authoring::publishing::Publication;
    let (mut app, temp, id) = fixture();
    let ctx = egui::Context::default();
    let path = temp.path().join("workshop");
    let manifest = pcbcraft_storage::library::publishing::read_manifest(&path).unwrap();
    let prepared = app.library_editors.parts[&id].session.prepare(app.session.libraries(), &manifest, 0, false, false).unwrap();
    pcbcraft_storage::library::publishing::publish(&path, &prepared).unwrap();
    let captured = prepared.reference();
    app.library_editors.publisher.recovery = Some(Request {
        document: id,
        source: "workshop".into(),
        path: path.to_str().unwrap().into(),
        generation: app.session.libraries().generation(),
        prepared: Capture::Part(std::sync::Arc::new(prepared)),
    });
    let mut later = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(part) = &mut later else { panic!() };
    part.name = "Edited during write".into();
    app.replace_part_document(id, 2, later).unwrap();
    app.publish_library_item(&ctx, id, 3, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    assert_eq!(std::fs::read_dir(path.join("parts")).unwrap().count(), 1);
    assert_eq!(app.library_editors.parts[&id].session.published(), Some(captured));
    assert!(app.library_editors.parts[&id].dirty());
    assert!(app.library_editors.publisher.recovery.is_none());
}
/// A native pre-write failure releases retry ownership and retains the complete unpublished form.
#[test]
fn failed_part_write_retains_draft_and_releases_worker() {
    let (mut app, temp, id) = fixture();
    let ctx = egui::Context::default();
    let record = app.library_editors.parts[&id].session.record().clone();
    std::fs::write(temp.path().join("workshop/parts"), "unrelated file").unwrap();
    app.publish_library_item(&ctx, id, 2, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    assert_eq!(app.library_editors.parts[&id].session.record(), &record);
    assert!(app.library_editors.parts[&id].dirty());
    assert!(app.library_editors.publisher.recovery.is_none());
    assert!(app.library_editors.notice().unwrap().contains("diagnostics"));
    assert_eq!(std::fs::read(temp.path().join("workshop/parts")).unwrap(), b"unrelated file");
}
/// Shared live commands retain one part session across split panes and reject stale updates.
#[test]
fn live_part_commands_share_document_and_keep_project_history() {
    let (mut app, _temp, id) = fixture();
    let ctx = egui::Context::default();
    let before = app.session.inspect();
    let root = app.views.active.id();
    let other = app.views.open_beside(root).unwrap();
    assert_eq!(app.views.view(other).unwrap().library.as_ref().unwrap().part_document, Some(id));
    let mut record = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(p) = &mut record else { panic!() };
    p.name = "Shared part".into();
    app.run_window_command(&ctx, "app.part_edit", &json!({"document":id,"expected_revision":2,"record":record})).unwrap();
    assert!(app.run_window_command(&ctx, "app.part_edit", &json!({"document":id,"expected_revision":2,"record":record})).is_err());
    let inspected = crate::window_commands::inspect_editor(&app);
    assert_eq!(inspected["library_editors"]["part_documents"][0]["document"], id);
    app.run_window_command(&ctx, "app.part_history", &json!({"document":id,"expected_revision":3})).unwrap();
    assert_eq!(app.library_editors.parts[&id].session.record().name(), "Original connector");
    assert_eq!(app.session.inspect(), before);
}

/// Publishing a drawing offers a source-part update, never a project update or an inferred remap.
#[test]
fn drawing_publication_opens_reviewed_part_reference_update() {
    use pcbcraft_engine::authoring::{Item, publishing as publisher};
    let (mut app, temp, id) = fixture();
    let ctx = egui::Context::default();
    let path = temp.path().join("workshop");
    let manifest = pcbcraft_storage::library::publishing::read_manifest(&path).unwrap();
    let symbol = app.session.libraries().symbol("basic-components", "connector", 1).unwrap();
    let mut item = Item::edit_symbol(&symbol).unwrap();
    item.library = "workshop".into();
    let first = publisher::prepare(Arc::new(item), &manifest, 0, None, false).unwrap();
    pcbcraft_storage::library::publishing::publish(&path, &first).unwrap();
    app.session.install_library_source("workshop", Arc::new(pcbcraft_storage::library::load(&path).unwrap())).unwrap();
    let mut record = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(p) = &mut record else { panic!() };
    p.symbol = Some(Reference { library: "workshop".into(), id: "connector".into(), revision: 1 });
    app.replace_part_document(id, 2, record).unwrap();
    app.publish_library_item(&ctx, id, 3, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    app.library_editors.close_part(id, false).unwrap();
    let mut item = (*first.item()).as_ref().clone();
    item.name = "Revised connector drawing".into();
    let second = publisher::prepare(Arc::new(item), &manifest, 1, Some(1), false).unwrap();
    pcbcraft_storage::library::publishing::publish(&path, &second).unwrap();
    app.session.install_library_source("workshop", Arc::new(pcbcraft_storage::library::load(&path).unwrap())).unwrap();
    let before = app.session.inspect();
    app.queue_part_dependency_updates(&second);
    assert_eq!(app.part_updates.len(), 1);
    app.open_part_dependency_update(&ctx, 0).unwrap();
    assert!(app.part_updates.is_empty());
    let id = app.views.active.library.as_ref().unwrap().part_document.unwrap();
    let doc = &app.library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.symbol.as_ref().unwrap().revision, second.item().revision);
    assert!(doc.dirty());
    assert_eq!(std::fs::read_dir(path.join("parts")).unwrap().count(), 1);
    assert_eq!(app.session.inspect(), before);
    app.publish_library_item(&ctx, id, doc.session.revision(), "workshop", false).unwrap();
    finish(&mut app, &ctx);
    assert_eq!(std::fs::read_dir(path.join("parts")).unwrap().count(), 2);
}

/// Real mapping choices preserve explicit pad ownership and disable publication until every
/// terminal is mapped again, using the same menus and engine validator as live clients.
#[test]
fn mapping_menu_edits_are_explicit_and_undoable() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (app, _temp, id) = fixture();
    let before = app.session.inspect();
    let mut h = Harness::builder().with_max_steps(64).with_size(egui::vec2(1800., 1100.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h.get_by_label("Terminal for pad 1").click();
    h.run();
    h.get_by_label("Unconnected").click();
    h.run();
    assert!(!egui::Popup::is_any_open(&h.ctx), "popup remained open after choosing Unconnected");
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.packages["header"].unconnected_pads, vec!["1"]);
    assert!(!part.packages["header"].mapping.contains_key("1"));
    assert!(doc.session.record().validate(h.state().session.libraries()).is_err());
    h.get_by_label("Terminal for pad 1").click();
    h.run();
    assert!(egui::Popup::is_any_open(&h.ctx), "mapping popup did not reopen");
    h.get_by_role_and_label(egui::Role::RadioButton, "1").click();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert!(
        doc.session.record().validate(h.state().session.libraries()).is_ok(),
        "state={} popup={}",
        doc.session.inspect(),
        egui::Popup::is_any_open(&h.ctx)
    );
    let revision = doc.session.revision();
    h.get_by_label("Undo").click();
    h.run();
    assert!(h.state().library_editors.parts[&id].session.revision() > revision);
    assert!(h.state().library_editors.parts[&id].session.record().validate(h.state().session.libraries()).is_err());
    assert_eq!(h.state().session.inspect(), before);
}

/// A physical-only definition uses the same durable publisher and can own an MPN without
/// inventing a schematic instance. Reopening native source bytes preserves the absent symbol.
#[test]
fn physical_part_and_orderable_publish_reopen_without_a_symbol() {
    let (mut app, temp, id) = fixture();
    let ctx = egui::Context::default();
    let before = app.session.inspect();
    app.choose_part_reference(id, 2, &ReferenceField::Symbol, None).unwrap();
    app.publish_library_item(&ctx, id, 3, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    let reference = app.library_editors.parts[&id].session.record().reference();
    let resolved = app.session.libraries().resolve_part(&reference).unwrap();
    assert!(resolved.symbol.is_none());
    assert_eq!(resolved.footprints.len(), 1);
    let orderable = app.new_orderable_document(&ctx, reference.clone()).unwrap();
    let mut record = app.library_editors.parts[&orderable].session.record().clone();
    let Record::Orderable(mpn) = &mut record else { panic!() };
    mpn.manufacturer = "Original test hardware".into();
    mpn.mpn = "PHYSICAL-ONLY".into();
    app.replace_part_document(orderable, 1, record).unwrap();
    app.publish_library_item(&ctx, orderable, 2, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    assert!(!app.library_editors.parts[&orderable].dirty());
    let source = Arc::new(pcbcraft_storage::library::load(&temp.path().join("workshop")).unwrap());
    assert!(source.problems().is_empty());
    assert_eq!(source.items().len(), 2);
    let mut catalog = Catalog::bundled();
    catalog.install("reopened", source).unwrap();
    assert!(catalog.resolve_part(&reference).unwrap().symbol.is_none());
    let mpn = app.library_editors.parts[&orderable].session.record().reference();
    catalog.manufacturer_part(&mpn).unwrap();
    assert_eq!(app.session.inspect(), before);
}

/// Compact hosts may open Preview without ever visiting Items; a physical-only part still
/// frames its actual footprint instead of relying on the form to initialize package selection.
#[test]
fn physical_preview_does_not_require_visiting_items() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _temp, id) = fixture();
    app.choose_part_reference(id, 2, &ReferenceField::Symbol, None).unwrap();
    assert!(app.library_editors.parts[&id].package.is_none());
    let mut h = Harness::builder().with_max_steps(64).with_size(egui::vec2(620., 850.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    crate::tests::click(&mut h, "Preview");
    assert!(h.query_all_by_label("Footprint preview").next().is_some());
    assert!(h.query_all_by_label("Symbol preview").next().is_none());
}
