use super::*;
use crate::{PcbApp, tests::click};
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

/// Open the real chooser at a requested viewport, leaving project state untouched.
fn editor(width: f32) -> egui_kittest::Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({}));
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(width, 800.))
        .with_max_steps(40)
        .build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h.key_press(Key::A);
    h.run();
    h
}

/// The default catalog is a large form with library navigation and preview, rather than a
/// small dock. Confirming starts a placement preview and leaves the full canvas unchanged.
#[test]
fn large_catalog_selects_libraries_and_confirms_before_placement() {
    let mut h = editor(1400.);
    let before = h.state().session.inspect();
    let canvas = h.get_by_label("Schematic canvas").rect();
    h.get_by_label("PCBcraft starter");
    h.get_by_label("All libraries");
    let close = h.get_by_label("Close Parts Catalog").rect();
    let place = h.get_by_label("Place").rect();
    assert!(close.right() - place.left() < 400.);
    assert!(close.top() + 300. < place.bottom());
    click(&mut h, "PCBcraft starter");
    click(&mut h, "Capacitor");
    assert_eq!(h.state().session.inspect(), before);
    assert!(h.state().views.active.schematic.tool.is_none());
    click(&mut h, "Place");
    assert!(!h.state().parts.open);
    assert_eq!(h.get_by_label("Schematic canvas").rect(), canvas);
    click(&mut h, "Schematic canvas");
    assert_eq!(h.state().session.project().unwrap().symbols()[0].definition.id, "capacitor");
}

/// Searches and library filters constrain current definitions; Escape cancels the chooser
/// while leaving its query available when A reopens it.
#[test]
fn catalog_search_scope_and_escape_preserve_document() {
    let mut h = editor(1400.);
    let before = h.state().session.inspect();
    h.get_by_label("Search parts").type_text("cap");
    h.run();
    h.get_by_label("Capacitor");
    assert!(h.query_by_label("Resistor").is_none());
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().parts.open);
    assert_eq!(h.state().session.inspect(), before);
    h.key_press(Key::A);
    h.run();
    assert_eq!(h.get_by_label("Search parts").value().as_deref(), Some("cap"));
    h.key_press(Key::Enter);
    h.run();
    assert!(!h.state().parts.open);
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::PlaceSymbol { .. })));
}

/// P selects the power collection in the same catalog, and a selected supply has an original
/// graphical preview and editable net name without the former Type/Preset property menu.
#[test]
fn power_uses_catalog_and_custom_net_names() {
    let mut h = editor(1400.);
    click(&mut h, "Cancel");
    h.key_press(Key::P);
    h.run();
    assert_eq!(h.state().parts.collection, Collection::Power);
    assert!(h.query_by_label("Type").is_none());
    assert!(h.query_by_label("Preset").is_none());
    click(&mut h, "+3V3");
    assert_eq!(h.get_by_label("Net name").value().as_deref(), Some("+3V3"));
    h.get_by_label("Net name").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.run();
    assert!(h.get_by_label("Place").accesskit_node().is_disabled());
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().parts.open);
    assert!(h.state().views.active.schematic.tool.is_none());
    h.get_by_label("Net name").click();
    h.get_by_label("Net name").type_text("+3V3_ANALOG");
    h.run();
    click(&mut h, "Place");
    assert!(matches!(h.state().views.active.schematic.tool,Some(crate::Tool::NetLabel{ref label,..}) if label.name=="+3V3_ANALOG"));
}

/// Compact hosts keep library selection and both result/preview pages inside the form.
/// Neither a closed chooser nor its legacy arrangement data reserves canvas space.
#[test]
fn compact_catalog_keeps_controls_reachable() {
    for width in [420., 700.] {
        let mut h = editor(width);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.));
        assert!(viewport.contains_rect(h.get_by_label("Close Parts Catalog").rect()));
        assert!(viewport.contains_rect(h.get_by_label("Place").rect()));
        h.get_by_label("Library");
        click(&mut h, "Cancel");
        assert!(!h.state().parts.rect().is_positive());
    }
}

/// A project edit while the catalog is open invalidates the captured placement authority.
#[test]
fn changed_project_cannot_receive_stale_catalog_choice() {
    let mut h = editor(1400.);
    let revision = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":revision})).unwrap();
    h.run();
    click(&mut h, "Place");
    assert!(!h.state().parts.open);
    assert!(h.state().views.active.schematic.tool.is_none());
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
}

/// Library management opens its existing merged form; it does not create a duplicate source UI.
#[test]
fn catalog_reaches_existing_library_manager() {
    let mut h = editor(1400.);
    click(&mut h, "Manage libraries…");
    assert!(!h.state().parts.open);
    assert!(matches!(h.state().dialog, crate::Dialog::LibrarySources));
    h.get_by_label("Close Library sources");
}

/// Catalog selection does not use display text as identity, and source filters use library IDs.
#[test]
fn search_preserves_original_starter_order_and_native_identity() {
    assert_eq!(matches("CAP")[0].id, "capacitor");
    assert!(matches("zzz").is_empty());
    assert_eq!(matches("j").len(), 2);
    let catalog = library::sources::Catalog::default();
    assert_eq!(entries("", &catalog).iter().filter(|e| e.in_collection(&Collection::Power)).count(), 8);
}

/// Registered libraries appear as selectable collections. If a chosen source is removed,
/// Place becomes unavailable rather than silently substituting the first remaining part.
#[test]
fn loaded_library_filters_and_removed_selection_do_not_alias_starter_parts() {
    for width in [1400., 700.] {
        let mut h = editor(width);
        click(&mut h, "Cancel");
        let documents: library::sources::Documents = [
            ("library.json".into(), include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-library/library.json")).into()),
            (
                "symbols/resistor.json".into(),
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-library/symbols/resistor.json")).into(),
            ),
        ]
        .into_iter()
        .collect();
        h.state_mut().session.install_library_source("workshop", Arc::new(library::sources::NativeSource::load(&documents).unwrap())).unwrap();
        h.key_press(Key::A);
        h.run();
        if width < 900. {
            click(&mut h, "Library");
        }
        click(&mut h, "Workshop demo");
        h.get_by_label("Workshop resistor · workshop/resistor r2");
        assert!(h.query_by_label("Resistor").is_none());
        assert!(matches!(h.state().parts.selected,Some(super::Key::Symbol(ref library,_,_)) if library=="workshop"));
        h.state_mut().session.apply(library::sources::REMOVE, &json!({"source":"workshop"})).unwrap();
        h.run();
        assert!(h.get_by_label("Place").accesskit_node().is_disabled());
        assert!(h.state().session.project().unwrap().symbols().is_empty());
        assert_eq!(h.state().parts.collection, Collection::Library("workshop".into()));
        h.key_press(Key::Enter);
        h.run();
        assert!(h.state().parts.open);
    }
}
