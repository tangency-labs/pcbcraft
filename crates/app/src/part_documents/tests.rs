use super::*;
use egui_kittest::kittest::NodeT;
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
    let symbol = app.session.libraries().latest_symbol("basic-components", "connector").unwrap();
    let id = app
        .new_part_document(
            &egui::Context::default(),
            Some(Reference { library: symbol.library.clone(), id: symbol.id.clone(), revision: symbol.revision() }),
        )
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
/// The fixture part bound to a synthetic symbol whose terminal 1 is a power input,
/// published in its own removable native source.
fn supply_fixture() -> (PcbApp, tempfile::TempDir, u64) {
    use pcbcraft_engine::{
        authoring::{Item, Kind as ItemKind, Primitive, publishing},
        library::sources::{Documents, NativeSource},
        schematic::ElectricalType,
    };

    let (mut app, temp, id) = fixture();
    let manifest = publishing::new_library("test-drawings", "Synthetic drawings", "MIT OR Apache-2.0").unwrap();
    let mut item = Item::new(ItemKind::Symbol).unwrap();
    item.library = "test-drawings".into();
    item.id = "synthetic-supply".into();
    let Some(Primitive::Pin { electrical_type, .. }) = item.objects.get_mut(&1) else { panic!("fixture needs a pin") };
    *electrical_type = ElectricalType::PowerIn;
    let prepared = publishing::prepare(Arc::new(item), &manifest, 0, None, false).unwrap();
    let mut documents = Documents::from([("library.json".into(), manifest)]);
    documents.extend(prepared.documents().clone());
    app.session.install_library_source("test-drawings", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();

    let symbol = prepared.item();
    let reference = Reference { library: symbol.library.clone(), id: symbol.id.clone(), revision: symbol.revision };
    let revision = app.library_editors.parts[&id].session.revision();
    app.choose_part_reference(id, revision, &ReferenceField::Symbol, Some(reference)).unwrap();
    (app, temp, id)
}
/// Losing a source with a pending bound leaves a visible way to discard just that text,
/// without undoing or rewriting the authored range or discarding unrelated typed edits.
#[test]
fn missing_supply_symbol_keeps_pending_text_visible_and_discardable() {
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::library::sources;

    let (mut app, _temp, id) = supply_fixture();
    let doc = &app.library_editors.parts[&id];
    let revision = doc.session.revision();
    let mut record = doc.session.record().clone();
    let Record::Part(part) = &mut record else { panic!() };
    part.set_supply_range("1", "4.5 V", "36 V").unwrap();
    app.replace_part_document(id, revision, record.clone()).unwrap();
    let doc = app.library_editors.parts.get_mut(&id).unwrap();
    let revision = doc.session.revision();
    doc.pending.insert("supply/1/min".into(), "unfinished".into());
    doc.pending.insert("test:field".into(), "other edit".into());
    // The form renders outside `app.frame`, so install the interface fonts the frame would.
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    assert!(h.query_by_label("Discard unavailable supply edits").is_none());
    h.state_mut().session.execute(sources::REMOVE, &json!({"source":"test-drawings"})).unwrap();
    h.run();
    h.get_by_label("Terminal 1 minimum: \"unfinished\"");
    h.get_by_label("Discard unavailable supply edits").click();
    h.run();
    assert!(h.query_by_label("Discard unavailable supply edits").is_none());
    let doc = h.state_mut().library_editors.parts.get_mut(&id).unwrap();
    assert_eq!(doc.pending, BTreeMap::from([("test:field".into(), "other edit".into())]));
    assert_eq!(doc.session.record(), &record);
    assert_eq!(doc.session.revision(), revision);
    doc.pending.clear();
    let ctx = h.ctx.clone();
    h.state_mut().begin_part_reference(&ctx, id, ReferenceField::Symbol).unwrap();
}

/// A supply range applied while the draft sits at the native size limit is refused as a
/// whole: the typed bounds stay pending and visible, and the draft and history are unchanged.
/// Once the draft fits, the same Apply clears the text and is a single undo step.
#[test]
fn rejected_supply_range_at_the_size_limit_keeps_the_entered_text() {
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::library::sources::MAX_FILE_BYTES;

    let (mut app, _temp, id) = supply_fixture();
    let doc = &app.library_editors.parts[&id];
    let revision = doc.session.revision();
    let unrated = doc.session.record().clone();
    let mut rated = unrated.clone();
    let Record::Part(part) = &mut rated else { panic!() };
    part.set_supply_range("1", "4.5 V", "36 V").unwrap();
    let growth = serde_json::to_vec(&rated).unwrap().len() - serde_json::to_vec(&unrated).unwrap().len();
    assert!(growth > 8, "a range must add more bytes than the slack below");
    // Pad a field the form keeps collapsed until the unrated draft is just under the limit.
    let mut padded = unrated.clone();
    let Record::Part(part) = &mut padded else { panic!() };
    part.license.push_str(&"x".repeat(MAX_FILE_BYTES - 8 - serde_json::to_vec(&unrated).unwrap().len()));
    let mut padded_rated = padded.clone();
    let Record::Part(part) = &mut padded_rated else { panic!() };
    part.set_supply_range("1", "4.5 V", "36 V").unwrap();
    assert!(serde_json::to_vec(&padded_rated).unwrap().len() > MAX_FILE_BYTES);
    app.replace_part_document(id, revision, padded.clone()).unwrap();
    let doc = app.library_editors.parts.get_mut(&id).unwrap();
    let revision = doc.session.revision();
    let entered = BTreeMap::from([("supply/1/max".to_string(), "36 V".to_string()), ("supply/1/min".to_string(), "4.5 V".to_string())]);
    doc.pending = entered.clone();

    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Device supply ranges").click();
    h.run();
    h.get_by_label("Apply range").click();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert_eq!(doc.pending, entered);
    assert_eq!(doc.session.record(), &padded);
    assert_eq!(doc.session.revision(), revision);
    assert!(doc.error.as_deref().is_some_and(|error| error.contains("1 MiB")), "{:?}", doc.error);
    h.get_by_label("Apply range");

    // Shrink the draft without touching the pending text; the retained entry now applies.
    let doc = h.state_mut().library_editors.parts.get_mut(&id).unwrap();
    doc.session.replace(revision, unrated.clone()).unwrap();
    let revision = doc.session.revision();
    h.run();
    h.get_by_label("Apply range").click();
    h.run();
    let doc = h.state_mut().library_editors.parts.get_mut(&id).unwrap();
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);
    assert_eq!(doc.error, None);
    assert_eq!(doc.session.record(), &rated);
    assert_eq!(doc.session.revision(), revision + 1);
    doc.session.history(revision + 1, false).unwrap();
    assert_eq!(doc.session.record(), &unrated);
}

/// Publishing and reopening a part at [`MAX_TERMINAL_RATINGS`] succeeds; applying one more
/// rating in the form leaves the draft and its undo history unchanged and keeps the typed bounds.
#[test]
fn terminal_rating_limit_publishes_reopens_and_keeps_typed_bounds_on_overflow() {
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::{
        authoring::{Action, Item, Kind as ItemKind, Primitive, Session as AuthoringSession, publishing},
        library::{
            parts::MAX_TERMINAL_RATINGS,
            sources::{Documents, NativeSource},
        },
        schematic::{ElectricalType, Point},
    };

    let (mut app, _temp, id) = fixture();
    let manifest = publishing::new_library("test-drawings", "Synthetic drawings", "MIT OR Apache-2.0").unwrap();
    let mut symbol = Item::new(ItemKind::Symbol).unwrap();
    symbol.library = "test-drawings".into();
    symbol.id = "max-supply".into();
    let mut symbol_editor = AuthoringSession::new(symbol, false).unwrap();
    symbol_editor.execute(0, Action::Delete { objects: vec![1, 3] }).unwrap();
    for index in 1..=MAX_TERMINAL_RATINGS {
        let grid = index as i64 - 1;
        symbol_editor
            .execute(
                symbol_editor.revision(),
                Action::Insert {
                    primitive: Primitive::Pin {
                        number: index.to_string(),
                        name: Some(format!("VIN{index}")),
                        electrical_type: ElectricalType::PowerIn,
                        alternates: Vec::new(),
                        at: Point { x_nm: (grid % 16 - 8) * 1_270_000, y_nm: (grid / 16 - 4) * 1_270_000 },
                        appearance: None,
                    },
                },
            )
            .unwrap();
    }
    let mut footprint = Item::new(ItemKind::Footprint).unwrap();
    footprint.library = "test-drawings".into();
    footprint.id = "max-supply-package".into();
    let Primitive::Pad { pad: template } = footprint.objects[&1].clone() else { panic!("fixture needs a pad") };
    let mut footprint_editor = AuthoringSession::new(footprint, false).unwrap();
    for index in 3..=MAX_TERMINAL_RATINGS {
        let mut pad = template.clone();
        pad.number = index.to_string();
        pad.center = Point { x_nm: (index as i64 % 16) * 1_270_000, y_nm: (index as i64 / 16 + 1) * 1_270_000 };
        footprint_editor.execute(footprint_editor.revision(), Action::Insert { primitive: Primitive::Pad { pad } }).unwrap();
    }
    let prepared_symbol = publishing::prepare(symbol_editor.snapshot(), &manifest, 0, None, false).unwrap();
    let prepared_footprint = publishing::prepare(footprint_editor.snapshot(), &manifest, 0, None, false).unwrap();
    let mut documents = Documents::from([("library.json".into(), manifest)]);
    documents.extend(prepared_symbol.documents().clone());
    documents.extend(prepared_footprint.documents().clone());
    app.session.install_library_source("test-drawings", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();

    let symbol_item = prepared_symbol.item();
    let symbol_ref = Reference { library: symbol_item.library.clone(), id: symbol_item.id.clone(), revision: symbol_item.revision };
    let footprint_item = prepared_footprint.item();
    let footprint_ref = Reference { library: footprint_item.library.clone(), id: footprint_item.id.clone(), revision: footprint_item.revision };
    let revision = app.library_editors.parts[&id].session.revision();
    app.choose_part_reference(id, revision, &ReferenceField::Symbol, Some(symbol_ref)).unwrap();
    let revision = app.library_editors.parts[&id].session.revision();
    let mut record = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(part) = &mut record else { panic!() };
    part.packages.clear();
    part.packages.insert(
        "max".into(),
        Package {
            name: "Max package".into(),
            footprint: footprint_ref,
            mapping: (1..=MAX_TERMINAL_RATINGS).map(|index| (index.to_string(), vec![index.to_string()])).collect(),
            unconnected_pads: Vec::new(),
        },
    );
    for index in 1..=MAX_TERMINAL_RATINGS {
        part.set_supply_range(&index.to_string(), "4.5 V", "36 V").unwrap();
    }
    app.replace_part_document(id, revision, record).unwrap();
    let ctx = egui::Context::default();
    let revision = app.library_editors.parts[&id].session.revision();
    app.publish_library_item(&ctx, id, revision, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    assert!(!app.library_editors.parts[&id].dirty(), "{:?}", app.library_editors.notice());
    let source = pcbcraft_storage::library::load(std::path::Path::new(&app.library_sources.registered[0].path)).unwrap();
    assert!(source.problems().is_empty());
    let reference = app.library_editors.parts[&id].session.record().reference();
    let reopened = app.session.libraries().resolve_part(&reference).unwrap();
    assert_eq!(reopened.definition.terminal_ratings.len(), MAX_TERMINAL_RATINGS);

    // Keep 128 ratings authored while leaving terminal 1 unrated so the form offers Apply range for it.
    let published = app.library_editors.parts[&id].session.record().clone();
    let mut at_limit = published.clone();
    let Record::Part(part) = &mut at_limit else { panic!() };
    part.set_supply_range("1", "", "").unwrap();
    part.set_supply_range(&(MAX_TERMINAL_RATINGS + 1).to_string(), "4.5 V", "36 V").unwrap();
    assert_eq!(part.terminal_ratings.len(), MAX_TERMINAL_RATINGS);
    let revision = app.library_editors.parts[&id].session.revision();
    app.replace_part_document(id, revision, at_limit.clone()).unwrap();
    let doc = app.library_editors.parts.get_mut(&id).unwrap();
    let revision = doc.session.revision();
    let entered = BTreeMap::from([("supply/1/max".to_string(), "36 V".to_string()), ("supply/1/min".to_string(), "4.5 V".to_string())]);
    doc.pending = entered.clone();

    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Device supply ranges").click();
    h.run();
    h.get_by_label("Apply range").click();
    h.run();
    h.get_by_label("Apply range");
    let doc = h.state_mut().library_editors.parts.get_mut(&id).unwrap();
    assert_eq!(doc.pending, entered);
    assert_eq!(doc.session.record(), &at_limit);
    assert_eq!(doc.session.revision(), revision);
    assert!(doc.error.as_deref().is_some_and(|error| error.contains(&MAX_TERMINAL_RATINGS.to_string())), "{:?}", doc.error);
    doc.session.history(revision, false).unwrap();
    assert_eq!(doc.session.record(), &published);
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
    let symbol = app.session.libraries().latest_symbol("basic-components", "connector").unwrap();
    let mut item = Item::from_symbol(&symbol).unwrap();
    item.id = symbol.id.clone();
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

/// The bottom action remains visible in a short host and reaches the real destination form;
/// choosing an existing writable library publishes through the ordinary host worker.
#[test]
fn bottom_publish_action_uses_the_visible_destination_flow() {
    use egui_kittest::{Harness, kittest::Queryable};
    for (width, height) in [(2735., 850.), (620., 650.)] {
        let (mut app, _temp, id) = fixture();
        app.choose_part_reference(id, 2, &ReferenceField::Symbol, None).unwrap();
        let mut h =
            Harness::builder().with_max_steps(64).with_size(egui::vec2(width, height)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        if width < 1050. {
            crate::tests::click(&mut h, "Items");
        }
        let publish = h.get_by_label("Publish…").rect();
        assert!(publish.top() > height * 0.7, "{publish:?}");
        assert!(publish.bottom() < height);
        assert!(h.query_all_by_label("Package").next().is_none());
        let before = h.state().library_editors.parts[&id].session.inspect();
        h.get_by_label("Publish…").click();
        h.run();
        assert!(matches!(h.state().dialog, Dialog::LibraryPublish(_)));
        h.get_by_label("Choose library…").click();
        h.run();
        h.get_by_role_and_label(egui::Role::RadioButton, "Workshop").click();
        h.run();
        h.get_by_label("Publish").click();
        h.run();
        let ctx = h.ctx.clone();
        finish(h.state_mut(), &ctx);
        h.run();
        assert!(!h.state().library_editors.parts[&id].dirty());
        assert_ne!(h.state().library_editors.parts[&id].session.inspect(), before);
    }
}

/// A fresh installation exposes creation directly, while cancellation returns to the same
/// captured publication and draft without any folder picker or publication side effect.
#[test]
fn publication_without_libraries_exposes_creation_and_preserves_cancel() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PcbApp::default();
    let id = app.new_part_document(&egui::Context::default(), None).unwrap();
    let before = app.library_editors.parts[&id].session.inspect();
    let mut h = Harness::builder().with_max_steps(64).with_size(egui::vec2(620., 700.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    crate::tests::click(&mut h, "Items");
    h.get_by_label("Choose category").click();
    h.run();
    assert!(h.query_all_by_label("No category definitions loaded").next().is_some());
    h.get_by_label("Cancel").click();
    h.run();
    h.get_by_label("Publish…").click();
    h.run();
    h.get_by_label("New library…").click();
    h.run();
    assert!(matches!(h.state().dialog, Dialog::NewLibrary(_)));
    h.get_by_label("Cancel").click();
    h.run();
    assert!(matches!(h.state().dialog, Dialog::LibraryPublish(_)));
    h.get_by_label("Cancel").click();
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().library_editors.parts[&id].session.inspect(), before);
}

/// Named tabs preserve drafts while browsing and return compact hosts to Items. Closing the
/// same dirty document uses the existing discard protection instead of removing it immediately.
#[test]
fn visible_document_tabs_return_to_retained_drafts() {
    use egui_kittest::{Harness, kittest::Queryable};
    for width in [2735., 620.] {
        let (app, _temp, id) = fixture();
        let before = app.library_editors.parts[&id].session.inspect();
        let mut h =
            Harness::builder().with_max_steps(64).with_size(egui::vec2(width, 850.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        h.get_by_label("Browse library").click();
        h.run();
        assert!(h.state().views.active.library.as_ref().unwrap().part_document.is_none());
        assert_eq!(h.state().library_editors.parts[&id].session.inspect(), before);
        h.get_by_label("Original connector").click();
        h.run();
        assert_eq!(h.state().views.active.library.as_ref().unwrap().part_document, Some(id));
        assert!(h.query_all_by_label("Choose footprint").next().is_some());
        assert_eq!(h.state().library_editors.parts[&id].session.inspect(), before);
        h.get_by_label("Close document").click();
        h.run();
        assert!(matches!(h.state().dialog,Dialog::LibraryClose(Some(owner)) if owner==id));
        assert!(h.state().library_editors.parts.contains_key(&id));
    }
}

/// Quit-time review must reveal a clean document whose copy publication still needs recovery,
/// rather than treating the absence of ordinary edits as permission to leave it hidden.
#[test]
fn review_edits_reveals_unconfirmed_clean_part_copy() {
    use crate::library_editors::publishing::{Capture, Request};
    let (mut app, temp, id) = fixture();
    let ctx = egui::Context::default();
    app.publish_library_item(&ctx, id, 2, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    assert!(!app.library_editors.parts[&id].dirty());
    let path = temp.path().join("workshop");
    let manifest = pcbcraft_storage::library::publishing::read_manifest(&path).unwrap();
    let prepared = app.library_editors.parts[&id].session.prepare(app.session.libraries(), &manifest, 0, false, true).unwrap();
    app.library_editors.publisher.recovery = Some(Request {
        document: id,
        source: "workshop".into(),
        path: path.to_str().unwrap().into(),
        generation: app.session.libraries().generation(),
        prepared: Capture::Part(Arc::new(prepared)),
    });
    let view = app.views.active.id().0;
    app.run_window_command(&ctx, "app.library_browse", &json!({"view":view,"source":"workshop"})).unwrap();
    assert!(app.views.active.library.as_ref().unwrap().part_document.is_none());
    assert_eq!(app.library_editors.publisher.unconfirmed(), Some(id));
    let before = app.session.snapshot().unwrap();
    app.review_library_edits(&ctx, None);
    assert_eq!(app.views.active.library.as_ref().unwrap().part_document, Some(id));
    assert_eq!(app.library_editors.publisher.unconfirmed(), Some(id));
    assert!(!app.library_editors.parts[&id].dirty());
    assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
}

/// An explicit click returns to Browse even when it selects the current source or project;
/// the retained part session is unchanged and can be selected again from its document tab.
#[test]
fn selected_browser_rows_return_from_retained_part_document() {
    use egui_kittest::{Harness, kittest::Queryable};
    for (params, label) in [
        (json!({"source":"workshop"}), "Workshop"),
        (json!({"source":"bundled-basic-components"}), "Basic components"),
        (json!({"scope":"project"}), "Project libraries"),
    ] {
        let (mut app, _temp, id) = fixture();
        let ctx = egui::Context::default();
        let mut params = params;
        params["view"] = json!(app.views.active.id().0);
        app.run_window_command(&ctx, "app.library_browse", &params).unwrap();
        app.place_part_document(&ctx, id).unwrap();
        let record = app.library_editors.parts[&id].session.inspect();
        let before = app.session.snapshot().unwrap();
        let mut h =
            Harness::builder().with_max_steps(64).with_size(egui::vec2(1800., 1100.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        h.get_by_role_and_label(egui::Role::Button, label).click();
        h.run();
        assert!(h.state().views.active.library.as_ref().unwrap().part_document.is_none(), "{label}");
        assert_eq!(h.state().library_editors.parts[&id].session.inspect(), record);
        h.get_by_role_and_label(egui::Role::Tab, "Original connector").click();
        h.run();
        assert_eq!(h.state().views.active.library.as_ref().unwrap().part_document, Some(id));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Updating a footprint retains both mapped and unconnected removed pads for explicit review.
/// Real removal and reassignment repair the draft without rebuilding its package, with Undo.
#[test]
fn removed_pad_assignments_are_visible_repairable_and_undoable() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _temp, id) = fixture();
    let mut old = (*app.session.libraries().footprint("basic-components", "header-1x02-254", 3).unwrap()).clone();
    old.library = "mapping-review".into();
    old.revision = 1;
    let mut extra = old.pads[1].clone();
    extra.number = "4".into();
    old.pads.push(extra);
    let mut new = old.clone();
    new.revision = 2;
    new.pads.retain(|pad| pad.number == "1");
    let mut replacement = old.pads[1].clone();
    replacement.number = "3".into();
    new.pads.push(replacement);
    let manifest = pcbcraft_engine::authoring::publishing::new_library("mapping-review", "Mapping review", "MIT OR Apache-2.0").unwrap();
    let documents = BTreeMap::from([
        ("library.json".into(), manifest),
        ("footprints/header-r1.json".into(), serde_json::to_string(&old).unwrap()),
        ("footprints/header-r2.json".into(), serde_json::to_string(&new).unwrap()),
    ]);
    let source = pcbcraft_engine::library::sources::NativeSource::load(&documents).unwrap();
    assert!(source.problems().is_empty(), "{:?}", source.problems());
    app.session.install_library_source("mapping-review", Arc::new(source)).unwrap();
    let old_reference = Reference { library: old.library.clone(), id: old.id.clone(), revision: 1 };
    let mut record = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(part) = &mut record else { panic!() };
    let package = part.packages.get_mut("header").unwrap();
    package.footprint = old_reference.clone();
    package.unconnected_pads.push("4".into());
    record.validate(app.session.libraries()).unwrap();
    let record = record.advance_dependency(Kind::Footprint, &old_reference, &Reference { revision: 2, ..old_reference.clone() }).unwrap();
    app.replace_part_document(id, 2, record.clone()).unwrap();
    let before = app.session.snapshot().unwrap();
    let mut h = Harness::builder().with_max_steps(64).with_size(egui::vec2(1800., 1300.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h.get_by_label("Remove assignment for pad 2").click();
    h.run();
    assert!(h.query_all_by_label("Remove assignment for pad 2").next().is_none());
    h.get_by_label("Undo").click();
    h.run();
    assert_eq!(h.state().library_editors.parts[&id].session.record(), &record);
    h.get_by_label("Redo").click();
    h.run();
    h.get_by_label("Remove assignment for pad 4").click();
    h.run();
    assert!(h.query_all_by_label("Remove assignment for pad 4").next().is_none());
    h.get_by_label("Terminal for pad 3").click();
    h.run();
    h.get_by_role_and_label(egui::Role::RadioButton, "2").click();
    h.run();
    let repaired = h.state().library_editors.parts[&id].session.record();
    repaired.validate(h.state().session.libraries()).unwrap();
    let Record::Part(part) = repaired else { panic!() };
    assert_eq!(part.packages["header"].mapping["2"], vec!["3"]);
    assert!(part.packages["header"].unconnected_pads.is_empty());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Render metadata controls in isolation: each valid edit reaches the session once, undo and
/// redo share its history, and blank text removes the authored value instead of storing it.
#[test]
fn metadata_fields_commit_undo_redo_and_clear() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (app, _temp, id) = fixture();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Search metadata").click();
    h.run();
    h.get_by_label("Description");
    h.get_by_label("Keywords");
    h.get_by_label("Add link");
    let revision = h.state().library_editors.parts[&id].session.revision();
    h.get_by_label("Description").click();
    h.run();
    h.get_by_label("Description").type_text("Low ESR ceramic");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.description.as_deref(), Some("Low ESR ceramic"));
    assert_eq!(part.format, pcbcraft_engine::library::parts::METADATA_PART_FORMAT);
    assert_eq!(doc.session.revision(), revision + 1);
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);

    h.get_by_label("Keywords").click();
    h.run();
    h.get_by_label("Keywords").type_text("low-esr\nceramic");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.keywords, ["low-esr", "ceramic"]);
    assert_eq!(doc.session.revision(), revision + 2);

    h.get_by_label("Undo").click();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert!(part.keywords.is_empty());
    assert_eq!(part.description.as_deref(), Some("Low ESR ceramic"));
    h.get_by_label("Redo").click();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.keywords, ["low-esr", "ceramic"]);

    h.get_by_label("Description").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.description, None);
    h.get_by_label("Keywords").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert!(part.keywords.is_empty());
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);
    assert_eq!(part.format, pcbcraft_engine::library::parts::PART_FORMAT);
}

/// Rejected bounds and non-HTTP links stay visible as pending text without moving the
/// published revision; correcting one field clears only that field's pending entry.
#[test]
fn metadata_rejects_oversized_text_and_bad_urls_without_publishing() {
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::library::parts::{MAX_DESCRIPTION_CHARS, MAX_KEYWORD_CHARS, MAX_KEYWORDS};
    let (mut app, temp, id) = fixture();
    let ctx = egui::Context::default();
    app.publish_library_item(&ctx, id, 2, "workshop", false).unwrap();
    finish(&mut app, &ctx);
    let folder = temp.path().join("workshop/parts");
    let file = std::fs::read_dir(&folder).unwrap().next().unwrap().unwrap().path();
    let bytes = std::fs::read(&file).unwrap();
    let published = app.library_editors.parts[&id].session.record().clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Search metadata").click();
    h.run();
    let revision = h.state().library_editors.parts[&id].session.revision();

    let oversized = "x".repeat(MAX_DESCRIPTION_CHARS + 1);
    h.get_by_label("Description").click();
    h.run();
    h.get_by_label("Description").type_text(&oversized);
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert_eq!(doc.pending.get("metadata/description"), Some(&oversized));
    assert_eq!(doc.session.record(), &published);
    assert_eq!(doc.session.revision(), revision);
    assert!(doc.error.as_deref().is_some_and(|error| error.contains(&MAX_DESCRIPTION_CHARS.to_string())), "{:?}", doc.error);
    assert!(h.get_by_label("Publish…").accesskit_node().is_disabled());
    h.get_by_label("Publish…").click();
    h.run();
    assert_eq!(std::fs::read(&file).unwrap(), bytes);

    let duplicate = (1..MAX_KEYWORDS).map(|n| format!("keyword-{n}")).collect::<Vec<_>>().join("\n");
    let duplicate = format!("{duplicate}\nKEYWORD-1");
    h.get_by_label("Keywords").click();
    h.run();
    h.get_by_label("Keywords").type_text(&duplicate);
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert_eq!(doc.pending.get("metadata/keywords"), Some(&duplicate));
    assert_eq!(doc.session.record(), &published);
    assert!(doc.error.as_deref().is_some_and(|error| error.contains("Duplicate keyword")), "{:?}", doc.error);
    let long = "y".repeat(MAX_KEYWORD_CHARS + 1);
    h.get_by_label("Keywords").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Keywords").type_text(&long);
    h.run();
    assert_eq!(h.state().library_editors.parts[&id].pending.get("metadata/keywords"), Some(&long));

    h.get_by_label("Description").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Description").type_text("Ceramic resistor");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert!(!doc.pending.contains_key("metadata/description"));
    assert_eq!(doc.pending.get("metadata/keywords"), Some(&long));
    assert!(h.get_by_label("Publish…").accesskit_node().is_disabled());

    h.get_by_label("Add link").click();
    h.run();
    h.get_by_label("Link label 1").click();
    h.run();
    h.get_by_label("Link label 1").type_text("Datasheet");
    h.run();
    h.get_by_label("Link URL 1").click();
    h.run();
    for bad_url in ["ftp://example.com/ds", "https://example.com:abc/datasheet"] {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Link URL 1").type_text(bad_url);
        h.run();
        let doc = &h.state().library_editors.parts[&id];
        assert_eq!(doc.pending.get("metadata/link/0/url").map(String::as_str), Some(bad_url));
        assert_eq!(doc.pending.get("metadata/link/0/label").map(String::as_str), Some("Datasheet"));
        let Record::Part(part) = doc.session.record() else { panic!() };
        assert!(part.links.is_empty());
        assert!(doc.error.as_deref().is_some_and(|error| error.contains("HTTP(S)")), "{:?}", doc.error);
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }

    h.get_by_label("Link URL 1").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Link URL 1").type_text("https://example.com/ds");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert!(!doc.pending.contains_key("metadata/link/0/url"));
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.links.len(), 1);
    assert_eq!(part.links[0].label, "Datasheet");
    assert_eq!(part.links[0].url, "https://example.com/ds");
    assert!(doc.pending.contains_key("metadata/keywords"));
    assert!(h.get_by_label("Publish…").accesskit_node().is_disabled());
}

/// Adding, removing and clearing link rows keeps pending text aligned with authored links
/// after indices shift, and the whole edit remains one revision with working undo/redo.
#[test]
fn link_rows_add_remove_clear_and_compact() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (app, _temp, id) = fixture();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Search metadata").click();
    h.run();
    h.get_by_label("Add link").click();
    h.run();
    h.get_by_label("Link label 1").click();
    h.run();
    h.get_by_label("Link label 1").type_text("Datasheet");
    h.run();
    h.get_by_label("Link URL 1").click();
    h.run();
    h.get_by_label("Link URL 1").type_text("https://example.com/ds");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.links.len(), 1);
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);
    let revision = doc.session.revision();

    h.get_by_label("Undo").click();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert!(part.links.is_empty());
    h.get_by_label("Redo").click();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.links.len(), 1);

    h.get_by_label("Add link").click();
    h.run();
    h.get_by_label("Link label 2").click();
    h.run();
    h.get_by_label("Link label 2").type_text("Application note");
    h.run();
    h.get_by_label("Link URL 2").click();
    h.run();
    h.get_by_label("Link URL 2").type_text("http://example.com/app");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.links.len(), 2);

    h.get_by_label("Remove link 1").click();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.links.len(), 1);
    assert_eq!(part.links[0].label, "Application note");
    assert_eq!(doc.session.revision(), revision + 4); // undo, redo, second link, removal

    h.get_by_label("Link label 1").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run();
    h.get_by_label("Link URL 1").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert!(part.links.is_empty());
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);
}

/// A valid link typed in a later row while earlier rows are blank keeps its row identity:
/// completing the earlier row inserts its own link ahead of the staged one instead of
/// overwriting it, and both links commit in row order.
#[test]
fn link_rows_stage_later_rows_and_commit_in_row_order() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _temp, id) = fixture();
    let doc = app.library_editors.parts.get_mut(&id).unwrap();
    doc.pending.insert("metadata/link/0/label".into(), String::new());
    doc.pending.insert("metadata/link/0/url".into(), String::new());
    doc.pending.insert("metadata/link/1/label".into(), String::new());
    doc.pending.insert("metadata/link/1/url".into(), String::new());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Search metadata").click();
    h.run();

    h.get_by_label("Link label 2").click();
    h.run();
    h.get_by_label("Link label 2").type_text("Datasheet");
    h.run();
    h.get_by_label("Link URL 2").click();
    h.run();
    h.get_by_label("Link URL 2").type_text("https://example.com/ds");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert_eq!(doc.pending.get("metadata/link/1/ready").map(String::as_str), Some("1"), "{:?}", doc.pending);
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert!(part.links.is_empty(), "a later row must not be appended ahead of a blank row");

    h.get_by_label("Link label 1").click();
    h.run();
    h.get_by_label("Link label 1").type_text("Application note");
    h.run();
    h.get_by_label("Link URL 1").click();
    h.run();
    h.get_by_label("Link URL 1").type_text("https://example.com/app");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.links.len(), 2, "{:?}", part.links);
    assert_eq!(part.links[0].label, "Application note");
    assert_eq!(part.links[1].label, "Datasheet");
}

/// Removing a blank pending row must not delete the accepted link that follows it, even when
/// the link sits at a lower index in the authored list while the blank row is still visible.
#[test]
fn removing_blank_link_row_keeps_the_accepted_link() {
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::library::parts::PartLink;
    let (mut app, _temp, id) = fixture();
    let revision = app.library_editors.parts[&id].session.revision();
    let mut record = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(part) = &mut record else { panic!() };
    part.links.push(PartLink { label: "Datasheet".into(), url: "https://example.com/ds".into() });
    app.replace_part_document(id, revision, record).unwrap();
    let doc = app.library_editors.parts.get_mut(&id).unwrap();
    doc.pending.insert("metadata/link/0/label".into(), String::new());
    doc.pending.insert("metadata/link/0/url".into(), String::new());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Search metadata").click();
    h.run();
    h.get_by_label("Remove link 1").click();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.links.len(), 1, "{:?}", part.links);
    assert_eq!(part.links[0].label, "Datasheet");
}

/// Trailing spaces remain in the active description buffer across frames while the field is
/// focused. The trimmed value is already committed, so publishing stays available; leaving the
/// field clears the buffer without undoing the commit.
#[test]
fn description_keeps_raw_buffer_while_editing_and_clears_it_on_focus_loss() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (app, _temp, id) = fixture();
    let raw = "Low ESR ceramic   ";
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Search metadata").click();
    h.run();
    h.get_by_label("Description").click();
    h.run();
    h.get_by_label("Description").type_text(raw);
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert_eq!(doc.pending.get("metadata/description").map(String::as_str), Some(raw), "{:?}", doc.pending);
    assert!(!doc.has_unapplied());
    assert!(!h.get_by_label("Publish…").accesskit_node().is_disabled());
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.description.as_deref(), Some("Low ESR ceramic"));

    h.get_by_label("Keywords").click();
    h.run();
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert!(!doc.pending.contains_key("metadata/description"), "{:?}", doc.pending);
    assert!(!doc.has_unapplied());
    assert!(!h.get_by_label("Publish…").accesskit_node().is_disabled());
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.description.as_deref(), Some("Low ESR ceramic"));
}

/// Enter in Keywords and a separately typed space in a link label survive the next paint.
#[test]
fn incremental_keyword_and_link_label_edits_keep_separators() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _temp, id) = fixture();
    let doc = &app.library_editors.parts[&id];
    let mut record = doc.session.record().clone();
    let Record::Part(part) = &mut record else { panic!() };
    part.links.push(pcbcraft_engine::library::parts::PartLink { label: "Data".into(), url: "https://example.com/datasheet".into() });
    app.replace_part_document(id, doc.session.revision(), record).unwrap();

    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Search metadata").click();
    h.run();
    h.get_by_label("Keywords").click();
    h.run();
    h.get_by_label("Keywords").type_text("precision");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert_eq!(doc.pending.get("metadata/keywords").map(String::as_str), Some("precision\n"));
    assert!(!doc.has_unapplied());
    h.get_by_label("Keywords").type_text("rail");
    h.run();

    h.get_by_label("Link label 1").click();
    h.run();
    h.key_press(egui::Key::End);
    h.get_by_label("Link label 1").type_text(" ");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert_eq!(doc.pending.get("metadata/link/0/label").map(String::as_str), Some("Data "));
    assert!(!doc.has_unapplied());
    h.get_by_label("Link label 1").type_text("sheet");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.keywords, ["precision", "rail"]);
    assert_eq!(part.links[0].label, "Data sheet");
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);
}

/// History and full record replacements discard applied buffers only on success.
#[test]
fn applied_metadata_buffers_do_not_hide_restored_records() {
    let (mut app, _temp, id) = fixture();
    let ctx = egui::Context::default();
    let doc = &app.library_editors.parts[&id];
    let before = doc.session.record().clone();
    let mut edited = before.clone();
    let Record::Part(part) = &mut edited else { panic!() };
    part.description = Some("Precision part".into());
    part.keywords = vec!["precision".into()];
    app.replace_part_document(id, doc.session.revision(), edited).unwrap();
    let doc = app.library_editors.parts.get_mut(&id).unwrap();
    let buffers = BTreeMap::from([("metadata/description".into(), "Precision part ".into()), ("metadata/keywords".into(), "precision\n".into())]);
    doc.pending = buffers.clone();
    let edited = doc.session.record().clone();
    let revision = doc.session.revision();
    assert!(!doc.has_unapplied());

    let history = |app: &mut PcbApp, expected_revision, redo| {
        app.part_document_command(&ctx, "app.part_history", &json!({"document":id,"expected_revision":expected_revision,"redo":redo}))
    };
    assert!(history(&mut app, revision - 1, false).is_err());
    assert!(app.replace_part_document(id, revision - 1, before.clone()).is_err());
    assert_eq!(app.library_editors.parts[&id].pending, buffers);
    history(&mut app, revision, false).unwrap();
    let doc = &app.library_editors.parts[&id];
    assert_eq!(doc.session.record(), &before);
    assert!(doc.pending.is_empty());
    history(&mut app, revision + 1, true).unwrap();
    let doc = app.library_editors.parts.get_mut(&id).unwrap();
    assert_eq!(doc.session.record(), &edited);
    doc.pending = buffers;
    app.replace_part_document(id, revision + 2, before.clone()).unwrap();
    let doc = &app.library_editors.parts[&id];
    assert_eq!(doc.session.record(), &before);
    assert!(doc.pending.is_empty());
}

/// A locally valid link stays pending when the bounded draft refuses the replacement, so a
/// size-limit rejection does not discard the typed label and URL. Retrying after the draft fits
/// applies the retained text in one revision.
#[test]
fn rejected_link_edit_keeps_label_and_url_until_the_draft_accepts_it() {
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::library::{parts::PartLink, sources::MAX_FILE_BYTES};

    let (mut app, _temp, id) = fixture();
    let revision = app.library_editors.parts[&id].session.revision();
    let base = app.library_editors.parts[&id].session.record().clone();
    let link = PartLink { label: "Datasheet".into(), url: "https://example.com/ds".into() };
    let mut with_link = base.clone();
    let Record::Part(part) = &mut with_link else { panic!() };
    part.links.push(link.clone());
    let growth = serde_json::to_vec(&with_link).unwrap().len() - serde_json::to_vec(&base).unwrap().len();
    assert!(growth > 8, "a link must add more bytes than the slack below");
    // Leave less slack than the link adds, so the draft fits now but refuses the typed link.
    let slack = growth / 2;
    let mut padded = base.clone();
    let Record::Part(part) = &mut padded else { panic!() };
    part.license.push_str(&"x".repeat(MAX_FILE_BYTES - slack - serde_json::to_vec(&base).unwrap().len()));
    assert!(serde_json::to_vec(&padded).unwrap().len() <= MAX_FILE_BYTES);
    app.replace_part_document(id, revision, padded.clone()).unwrap();
    let revision = app.library_editors.parts[&id].session.revision();

    let mut h = Harness::builder()
        .with_size(egui::vec2(1000., 900.))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
    h.run();
    h.get_by_label("Expand Search metadata").click();
    h.run();
    h.get_by_label("Add link").click();
    h.run();
    h.get_by_label("Link label 1").click();
    h.run();
    h.get_by_label("Link label 1").type_text("Datasheet");
    h.run();
    h.get_by_label("Link URL 1").click();
    h.run();
    h.get_by_label("Link URL 1").type_text("https://example.com/ds");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert_eq!(doc.pending.get("metadata/link/0/label").map(String::as_str), Some("Datasheet"), "{:?}", doc.pending);
    assert_eq!(doc.pending.get("metadata/link/0/url").map(String::as_str), Some("https://example.com/ds"));
    assert!(doc.has_unapplied());
    assert_eq!(doc.session.record(), &padded);
    assert_eq!(doc.session.revision(), revision);
    assert!(doc.error.as_deref().is_some_and(|error| error.contains("1 MiB")), "{:?}", doc.error);

    let doc = h.state_mut().library_editors.parts.get_mut(&id).unwrap();
    doc.session.replace(revision, base).unwrap();
    h.run();
    h.get_by_label("Link URL 1").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Link URL 1").type_text("https://example.com/ds/v2");
    h.run();
    let doc = &h.state().library_editors.parts[&id];
    assert!(doc.pending.is_empty(), "{:?}", doc.pending);
    assert_eq!(doc.error, None);
    let Record::Part(part) = doc.session.record() else { panic!() };
    assert_eq!(part.links.len(), 1);
    assert_eq!(part.links[0].label, "Datasheet");
    assert_eq!(part.links[0].url, "https://example.com/ds/v2");
}

/// A valid description or keyword edit that the bounded draft refuses must keep the raw text in
/// `pending`; the applied-key cleanup is only allowed after `Session::replace` succeeds.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rejected_keyword_and_description_edits_keep_their_typed_text() {
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::library::sources::MAX_FILE_BYTES;

    for field in ["Description", "Keywords"] {
        let (mut app, _temp, id) = fixture();
        let revision = app.library_editors.parts[&id].session.revision();
        let base = app.library_editors.parts[&id].session.record().clone();
        let mut candidate = base.clone();
        let Record::Part(part) = &mut candidate else { panic!() };
        match field {
            "Description" => part.description = Some("A deliberately long low-ESR ceramic description".into()),
            _ => part.keywords = vec!["specific-keyword".into()],
        }
        let growth = serde_json::to_vec(&candidate).unwrap().len() - serde_json::to_vec(&base).unwrap().len();
        assert!(growth > 8, "{field} must add more bytes than the slack below");
        let slack = growth / 2;
        let mut padded = base.clone();
        let Record::Part(part) = &mut padded else { panic!() };
        part.license.push_str(&"x".repeat(MAX_FILE_BYTES - slack - serde_json::to_vec(&base).unwrap().len()));
        assert!(serde_json::to_vec(&padded).unwrap().len() <= MAX_FILE_BYTES);
        app.replace_part_document(id, revision, padded.clone()).unwrap();

        let mut h = Harness::builder()
            .with_size(egui::vec2(1000., 900.))
            .with_context_setup(pcbcraft_chrome::install)
            .build_ui_state(move |ui, app: &mut PcbApp| app.part_document_form(ui, id), app);
        h.run();
        h.get_by_label("Expand Search metadata").click();
        h.run();
        h.get_by_label(field).click();
        h.run();
        h.get_by_label(field).type_text(match field {
            "Description" => "A deliberately long low-ESR ceramic description",
            _ => "specific-keyword",
        });
        h.run();
        let doc = &h.state().library_editors.parts[&id];
        let key = match field {
            "Description" => "metadata/description",
            _ => "metadata/keywords",
        };
        assert!(doc.pending.contains_key(key), "{field}: {:?}", doc.pending);
        assert_eq!(doc.session.record(), &padded, "{field} must not move the refused draft");
        assert!(doc.error.as_deref().is_some_and(|error| error.contains("1 MiB")), "{field}: {:?}", doc.error);
    }
}
