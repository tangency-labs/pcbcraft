use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;
use std::sync::Arc;

/// Generate thousands of original native definitions within ordinary source/file limits.
/// Repeated names and two revisions per needle exercise identity rather than display-name choice.
fn large_catalog() -> Catalog {
    use pcbcraft_engine::authoring::{Item, Kind as DrawingKind, publishing};
    let mut catalog = Catalog::bundled();
    let original = Item::new(DrawingKind::Symbol).unwrap();
    for library in 0..16 {
        let manifest = publishing::new_library(&format!("scale-{library:02}"), &format!("Test collection {library:02}"), &original.license).unwrap();
        let mut documents = BTreeMap::from([("library.json".into(), manifest.clone())]);
        for index in 0..200 {
            let mut item = original.clone();
            item.id = format!("item-{index:03}");
            item.name = if index == 0 { "Needle connector".into() } else { format!("Original symbol {index:03}") };
            let prepared = publishing::prepare(Arc::new(item.clone()), &manifest, 0, None, false).unwrap();
            documents.extend(prepared.documents().clone());
            if index == 0 {
                let prepared = publishing::prepare(Arc::new(item), &manifest, 1, Some(1), false).unwrap();
                documents.extend(prepared.documents().clone());
            }
        }
        catalog
            .install(&format!("scale-source-{library:02}"), Arc::new(pcbcraft_engine::library::sources::NativeSource::load(&documents).unwrap()))
            .unwrap();
    }
    catalog
}
/// Thousands of installed records remain searchable, while rendering and copying are page-bounded.
#[test]
fn thousands_of_records_filter_cache_paginate_and_keep_revisions_explicit() {
    let mut catalog = large_catalog();
    let mut state = State::new(1, 1, ReferenceField::Symbol, HostId::ROOT, None, &catalog);
    let existing = State::new(
        2,
        1,
        ReferenceField::Symbol,
        HostId::ROOT,
        Some(Reference { library: "scale-07".into(), id: "item-000".into(), revision: 1 }),
        &catalog,
    );
    assert!(existing.older);
    assert_eq!(existing.candidate(&catalog).unwrap().revision, 1);
    assert!(state.rows.len() > 3200);
    assert_eq!(state.page().len(), PAGE_SIZE);
    let rows = state.rows.as_ptr();
    state.refresh(&catalog);
    assert_eq!(rows, state.rows.as_ptr());
    state.page = state.rows.len() / PAGE_SIZE;
    assert!(state.page().len() <= PAGE_SIZE);
    state.query = "Needle".into();
    state.refresh(&catalog);
    assert_eq!(state.rows.len(), 16);
    assert!(state.rows.iter().all(|row| row.key.revision == 2));
    state.library = "scale-07".into();
    state.refresh(&catalog);
    assert_eq!(state.rows.len(), 1);
    state.select(0);
    assert_eq!(state.candidate(&catalog).unwrap().library, "scale-07");
    state.older = true;
    state.refresh(&catalog);
    assert!(state.selected.is_none());
    assert_eq!(state.rows.len(), 2);
    assert_eq!(state.rows.iter().map(|r| r.key.revision).collect::<Vec<_>>(), vec![2, 1]);
    catalog.remove("scale-source-07").unwrap();
    state.refresh(&catalog);
    assert!(state.rows.is_empty());
    assert!(state.selected.is_none());
}
/// Search and cancel leave draft/project history untouched; choosing an exact match uses the
/// same shared operation as live clients, and works at compact and wide sizes.
#[test]
fn search_preview_keyboard_choose_and_cancel_use_real_controls() {
    for width in [1800., 620.] {
        let (app, _temp, id) = super::super::tests::fixture();
        let before = app.session.inspect();
        let original = app.library_editors.parts[&id].session.inspect();
        let mut h =
            Harness::builder().with_max_steps(64).with_size(egui::vec2(width, 950.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        if width < 1050. {
            crate::tests::click(&mut h, "Items");
        }
        h.get_by_label("Choose symbol").click();
        h.run();
        assert!(matches!(h.state().dialog, Dialog::PartReference(_)));
        h.get_by_label("Search library items").type_text("resistor");
        h.run();
        let Dialog::PartReference(state) = &h.state().dialog else { panic!() };
        assert!(state.rows.iter().all(|row| row.name.to_lowercase().contains("resistor")));
        assert!(state.selected.is_none());
        h.get_by_label("Cancel").click();
        h.run();
        assert_eq!(h.state().library_editors.parts[&id].session.inspect(), original);
        h.get_by_label("Choose symbol").click();
        h.run();
        h.get_by_label("Search library items").type_text("resistor");
        h.run();
        h.key_press(egui::Key::ArrowDown);
        h.run();
        let Dialog::PartReference(state) = &h.state().dialog else { panic!() };
        let chosen = state.candidate(h.state().session.libraries()).unwrap();
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(h.state().library_editors.parts[&id].session.record().reference_field(&ReferenceField::Symbol).unwrap(), Some(chosen));
        assert_eq!(h.state().session.inspect(), before);
    }
}
/// A stale chooser, removed package or changed source cannot replace current relationships.
#[test]
fn exact_reference_selection_rejects_stale_and_missing_candidates_atomically() {
    let (mut app, _temp, id) = super::super::tests::fixture();
    let ctx = egui::Context::default();
    app.begin_part_reference(&ctx, id, ReferenceField::Symbol).unwrap();
    let Dialog::PartReference(state) = std::mem::take(&mut app.dialog) else { panic!() };
    let mut newer = app.library_editors.parts[&id].session.record().clone();
    let Record::Part(p) = &mut newer else { panic!() };
    p.name = "Newer draft".into();
    app.replace_part_document(id, 2, newer).unwrap();
    let before = app.library_editors.parts[&id].session.inspect();
    let resistor = Reference { library: "basic-components".into(), id: "resistor".into(), revision: 1 };
    assert!(app.choose_part_reference(id, state.revision, &state.field, Some(resistor)).is_err());
    assert_eq!(app.library_editors.parts[&id].session.inspect(), before);
    assert!(
        app.choose_part_reference(
            id,
            3,
            &ReferenceField::Footprint { package: "missing".into() },
            Some(Reference { library: "basic-components".into(), id: "header-1x02-254".into(), revision: 3 })
        )
        .is_err()
    );
    assert_eq!(app.library_editors.parts[&id].session.inspect(), before);
    assert!(
        app.run_window_command(
            &ctx,
            "app.part_reference",
            &json!({"document":id,"expected_revision":3,"field":{"field":"symbol"},"reference":{"library":"missing","id":"missing","revision":1}})
        )
        .is_err()
    );
    assert_eq!(app.library_editors.parts[&id].session.inspect(), before);
}
/// An explicit live footprint choice clears the old map, keeps package identity and is undoable.
#[test]
fn live_reference_choice_and_undo_share_gui_field_semantics() {
    let (mut app, _temp, id) = super::super::tests::fixture();
    let ctx = egui::Context::default();
    let before = app.session.inspect();
    let footprint = app
        .session
        .libraries()
        .search("", Some(Kind::Footprint))
        .into_iter()
        .find(|(_, item)| item.key.id != "header-1x02-254")
        .unwrap()
        .1
        .key
        .clone();
    app.run_window_command(&ctx,"app.part_reference",&json!({"document":id,"expected_revision":2,"field":{"field":"footprint","package":"header"},"reference":{"library":footprint.library,"id":footprint.id,"revision":footprint.revision}})).unwrap();
    let Record::Part(part) = app.library_editors.parts[&id].session.record() else { panic!() };
    assert!(part.packages["header"].mapping.is_empty());
    assert_eq!(part.packages["header"].footprint.id, footprint.id);
    app.run_window_command(&ctx, "app.part_history", &json!({"document":id,"expected_revision":3})).unwrap();
    let Record::Part(part) = app.library_editors.parts[&id].session.record() else { panic!() };
    assert_eq!(part.packages["header"].mapping.len(), 2);
    assert_eq!(app.session.inspect(), before);
}
