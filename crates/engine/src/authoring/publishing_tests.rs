use super::*;
use publishing::{new_library, prepare};

/// Native source limits remain editable without validating a temporary fork label first.
#[test]
fn footprint_reconstruction_preserves_maximum_native_source_text() {
    let base = Item::new(Kind::Footprint).unwrap().footprint().unwrap();
    for length in [1000, 1024, 2048] {
        let mut definition = base.clone();
        definition.source.reference = "a".repeat(length);
        let item = Item::edit_footprint(&definition).unwrap();
        assert_eq!(item.source, definition.source.reference);
        assert_eq!(item.footprint().unwrap().source.reference, definition.source.reference);
        assert!(Item::from_footprint(&definition).unwrap().source.len() <= 2048);
    }
}

/// Copy attribution uses UTF-8 byte limits including its prefix, retaining valid code points.
#[test]
fn edit_copy_bounds_multibyte_source_and_preserves_the_original() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        let mut item = Item::new(kind).unwrap();
        item.source = "é".repeat(500);
        let before = item.encode().unwrap();
        let copy = item.edit_copy().unwrap();
        assert!(copy.source.len() <= 1024);
        assert!(copy.source.starts_with("Fork of "));
        copy.validate().unwrap();
        assert_eq!(item.encode().unwrap(), before);
    }
}

/// Legacy bundled definitions without embedded attribution retain the bundled license on copy.
#[test]
fn legacy_starter_copy_infers_recorded_license() {
    let symbol = library::compatibility::starter().symbol("connector").unwrap();
    assert!(symbol.provenance.is_none());
    let copy = Item::from_symbol(symbol).unwrap();
    assert_eq!(copy.license, "MIT OR Apache-2.0");
    let manifest = new_library("workshop", "Workshop", &copy.license).unwrap();
    prepare(Arc::new(copy), &manifest, 0, None, false).unwrap();
}

/// Preparation is a read-only validation boundary, shared with transported MCP documents.
#[test]
fn publication_preparation_does_not_save_or_touch_project_history() {
    let item = Arc::new(Item::new(Kind::Symbol).unwrap());
    let manifest = new_library("workshop", "Workshop", &item.license).unwrap();
    let mut editor = Session::new((*item).clone(), false).unwrap();
    let mut project = crate::Session::default();
    project.apply("project.new", &json!({})).unwrap();
    let before = project.inspect();
    let prepared = prepare(item.clone(), &manifest, 0, None, false).unwrap();
    let result = project
        .execute(
            library::sources::PREPARE_PUBLICATION,
            &json!({
                "item":item.as_ref(),"documents":{"library.json":manifest}
            }),
        )
        .unwrap();
    assert_eq!(prepared.inspect(), result);
    assert_eq!(before, project.inspect());
    assert!(editor.dirty());
    assert!(editor.published().is_none());
    editor.confirm_publication(&prepared);
    assert!(!editor.dirty());
    assert!(!editor.publication_dirty());
    assert_eq!(editor.snapshot().library, "workshop");
    assert_eq!(editor.snapshot().id, item.id);
    assert_eq!(editor.published().unwrap().revision, 1);
    assert!(
        project
            .execute(library::sources::PREPARE_PUBLICATION, &json!({"item":item.as_ref(),"documents":{"library.json":manifest},"path":"/tmp/unsafe"}))
            .is_err()
    );
}

/// Late completion binds the new address across Undo/Redo while retaining later geometry as dirty.
#[test]
fn publication_completion_preserves_newer_edits_and_history_identity() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        let item = Item::new(kind).unwrap();
        let manifest = new_library("workshop", "Workshop", &item.license).unwrap();
        let mut editor = Session::new(item, false).unwrap();
        let prepared = prepare(editor.snapshot(), &manifest, 25, None, true).unwrap();
        editor.execute(0, Action::Delete { objects: vec![3] }).unwrap();
        let geometry = editor.snapshot().objects.clone();
        editor.confirm_publication(&prepared);
        assert_eq!(editor.snapshot().objects, geometry);
        assert!(editor.dirty());
        assert_eq!(editor.snapshot().library, "workshop");
        editor.execute(1, Action::Undo).unwrap();
        assert_eq!(editor.snapshot().library, "workshop");
        assert_eq!(editor.snapshot().id, prepared.item().id);
        editor.execute(2, Action::Redo).unwrap();
        let next = prepare(editor.snapshot(), &manifest, 26, Some(26), false).unwrap();
        assert!(next.item().revision > 26);
        assert_eq!(next.item().id, prepared.item().id);
    }
}

/// A later known revision requires consent and revision allocation never rewinds or wraps.
#[test]
fn older_base_requires_confirmation_and_allocation_is_bounded() {
    let item = Arc::new(Item::new(Kind::Symbol).unwrap());
    let manifest = new_library("workshop", "Workshop", &item.license).unwrap();
    assert!(prepare(item.clone(), &manifest, 7, Some(1), false).unwrap_err().contains("intervening changes"));
    assert_eq!(prepare(item.clone(), &manifest, 7, Some(1), true).unwrap().item().revision, 8);
    assert!(prepare(item, &manifest, u32::MAX, Some(1), true).is_err());
}

/// Published reconstruction preserves legacy graphics instead of performing Edit copy's lead migration.
#[test]
fn published_symbol_reconstruction_is_repeatable_and_preserves_terminal_geometry() {
    let documents = library::sources::Documents::from([
        ("library.json".into(), new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap()),
        ("symbols/connector.json".into(), include_str!("../../../../libraries/basic-components/symbols/connector.json").into()),
    ]);
    let source = library::sources::NativeSource::load(&documents).unwrap();
    let original = source.items()[0].symbol().unwrap();
    let item = Item::edit_symbol(&original).unwrap();
    assert_eq!(item.encode().unwrap(), Item::edit_symbol(&original).unwrap().encode().unwrap());
    assert_eq!(item.symbol().unwrap().units, original.units);
    assert_eq!((&item.library, &item.id, item.revision), (&original.library, &original.id, original.revision()));
    let copy = Item::from_symbol(&original).unwrap();
    assert_eq!(copy.library, "user");
    assert_ne!(copy.id, original.id);
}

/// Reserved external manifests fail without preventing already pinned project records from decoding.
#[test]
fn reserved_destinations_and_external_project_manifests_are_refused() {
    let item = Arc::new(Item::new(Kind::Footprint).unwrap());
    for id in ["user", "project-reserved", "basic-components", "starter", "power"] {
        let manifest = json!({"format":"pcbcraft-library/1","id":id,"name":"Reserved","license":"MIT"}).to_string();
        assert!(prepare(item.clone(), &manifest, 0, None, false).is_err());
        if id.starts_with("project-") {
            assert!(library::sources::NativeSource::load(&library::sources::Documents::from([("library.json".into(), manifest)])).is_err());
        }
    }
    let mut pinned = (*item).clone();
    pinned.library = "project-retained".into();
    assert!(pinned.native().is_ok());
}

/// A symbol's manifest attribution must not silently change the license recorded in its draft.
#[test]
fn symbol_publication_does_not_silently_relicense() {
    let item = Arc::new(Item::new(Kind::Symbol).unwrap());
    let manifest = new_library("workshop", "Workshop", "MIT").unwrap();
    assert!(prepare(item, &manifest, 0, None, false).unwrap_err().contains("license"));
}

/// Save As binds the same editor to a derivative only after confirmation; Undo retains that
/// new address and fields captured for the original identity cannot commit into it.
#[test]
fn publish_copy_rebinds_history_only_after_confirmation() {
    let mut item = Item::new(Kind::Symbol).unwrap();
    item.library = "original".into();
    let mut session = Session::new(item, true).unwrap();
    session.bind_published();
    session.execute(0, Action::Delete { objects: vec![3] }).unwrap();
    let original = session.snapshot();
    let manifest = new_library("destination", "Destination", &original.license).unwrap();
    let prepared = publishing::prepare_copy(original.clone(), &manifest).unwrap();
    assert!(Arc::ptr_eq(&session.snapshot(), &original));
    assert_eq!(session.published().unwrap().library, "original");
    assert_ne!(prepared.item().id, original.id);
    assert_eq!(prepared.item().name, original.name);
    session.confirm_publication(&prepared);
    assert!(!session.dirty());
    assert_eq!(session.snapshot().library, "destination");
    assert_eq!(session.snapshot().objects, original.objects);
    assert!(session.execute(1, Action::Undo).is_err());
    session.execute(2, Action::Undo).unwrap();
    assert_eq!(session.snapshot().library, "destination");
    assert_eq!(session.snapshot().id, prepared.item().id);
    assert!(session.snapshot().objects.contains_key(&3));
    let next = prepare(session.snapshot(), &manifest, 1, Some(1), false).unwrap();
    assert!(next.item().revision > session.snapshot().allocation.revision);
}
