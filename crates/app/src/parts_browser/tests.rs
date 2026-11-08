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
    for (width, compact) in [(1400., false), (700., false), (1400., true)] {
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
        if compact {
            click(&mut h, "Dock Parts Catalog");
        }
        if width < 900. || compact {
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

/// Dock/expand share one browsing state, and a fresh compact catalog starts beneath Browser.
#[test]
fn switching_forms_keeps_library_query_selection_and_dock_position() {
    let mut h = editor(1400.);
    click(&mut h, "PCBcraft starter");
    click(&mut h, "Search parts");
    h.get_by_label("Search parts").type_text("cap");
    h.run();
    let selected = h.state().parts.selected.clone();
    let before = h.state().session.inspect();
    let canvas = h.get_by_label("Schematic canvas").rect();
    click(&mut h, "Dock Parts Catalog");
    assert!(!h.state().parts.is_modal());
    assert_eq!(h.get_by_label("Search parts").value().as_deref(), Some("cap"));
    assert_eq!(h.state().parts.selected, selected);
    let card = h.state().parts.rect();
    let browser = h.state().browser.rect();
    assert!(card.top() >= browser.bottom());
    assert!((card.left() - browser.left()).abs() < 2.);
    assert_eq!(h.get_by_label("Schematic canvas").rect(), canvas);
    let position = h.state().parts.panel.saved_state();
    click(&mut h, "Expand Parts Catalog");
    assert!(h.state().parts.is_modal());
    h.get_by_label("Preview Capacitor");
    assert_eq!(h.state().parts.collection, Collection::Library("starter".into()));
    assert_eq!(h.state().parts.selected, selected);
    assert_eq!(h.get_by_label("Search parts").value().as_deref(), Some("cap"));
    click(&mut h, "Dock Parts Catalog");
    assert_eq!(h.state().parts.panel.saved_state(), position);
    assert_eq!(h.state().session.inspect(), before);
    click(&mut h, "Parts Catalog position");
    click(&mut h, "Float");
    let floating = h.state().parts.panel.saved_state();
    click(&mut h, "Expand Parts Catalog");
    click(&mut h, "Dock Parts Catalog");
    assert_eq!(h.state().parts.panel.saved_state(), floating);
    click(&mut h, "Close Parts Catalog");
    h.key_press(Key::A);
    h.run();
    assert_eq!(h.state().parts.mode, Mode::Compact);
    assert_eq!(h.state().parts.panel.saved_state(), floating);
}

/// Compact browsing leaves the canvas usable, remains open after Place, and yields search
/// focus on Escape before ordinary canvas shortcuts consume keys.
#[test]
fn compact_placement_and_keyboard_focus_leave_the_catalog_open() {
    let mut h = editor(1400.);
    click(&mut h, "Dock Parts Catalog");
    click(&mut h, "Capacitor");
    click(&mut h, "Place");
    assert!(h.state().parts.open);
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::PlaceSymbol { .. })));
    click(&mut h, "Schematic canvas");
    assert_eq!(h.state().session.project().unwrap().symbols()[0].definition.id, "capacitor");
    assert!(h.state().parts.open);
    h.key_press(Key::Escape);
    h.run();
    h.key_press(Key::A);
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().parts.open);
    h.key_press(Key::W);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::Wire { .. })));
}

/// Supply selection and custom names survive both presentation switches and compact placement.
#[test]
fn power_name_survives_round_trip_and_compact_placement() {
    let mut h = editor(1400.);
    click(&mut h, "Cancel");
    h.key_press(Key::P);
    h.run();
    click(&mut h, "+3V3");
    click(&mut h, "Net name");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Net name").type_text("V_ANALOG");
    h.run();
    click(&mut h, "Dock Parts Catalog");
    assert_eq!(h.get_by_label("Net name").value().as_deref(), Some("V_ANALOG"));
    assert!(h.state().parts.rect().contains_rect(h.get_by_label("+3V3").rect()));
    click(&mut h, "Expand Parts Catalog");
    h.get_by_label("Preview +3V3");
    assert_eq!(h.get_by_label("Net name").value().as_deref(), Some("V_ANALOG"));
    click(&mut h, "Dock Parts Catalog");
    click(&mut h, "Place");
    assert!(h.state().parts.open);
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::NetLabel{ref label,..}) if label.name=="V_ANALOG"));
}

/// Compact visibility/position persist; the previous arrangement vocabulary remains readable
/// and an expanded modal never reopens from saved presentation data.
#[test]
fn compact_state_restores_without_restoring_modal_or_input_authority() {
    let mut h = editor(1400.);
    let expanded = h.state().parts.saved_state();
    assert!(!PartsBrowser::from_state(&expanded).open);
    let mut previous = serde_json::to_value(&expanded).unwrap();
    previous.as_object_mut().unwrap().remove("mode");
    previous.as_object_mut().unwrap().remove("repeat");
    let previous: PartsState = serde_json::from_value(previous).unwrap();
    assert!(!PartsBrowser::from_state(&previous).open);
    assert!(!PartsBrowser::from_state(&previous).repeats());
    click(&mut h, "Dock Parts Catalog");
    let saved = h.state().parts.saved_state();
    let restored = PartsBrowser::from_state(&saved);
    assert!(restored.open);
    assert_eq!(restored.mode, Mode::Compact);
    assert_eq!(restored.panel.saved_state(), saved.panel);
    assert!(restored.source.is_none());
    assert!(restored.focus_ids.is_empty());
}

/// Live commands use the GUI mode handler and validate addresses/modes before changing anything.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn catalog_mode_command_matches_gui_and_rejects_invalid_modes_atomically() {
    let mut h = editor(1400.);
    let ctx = h.ctx.clone();
    let before = h.state().session.inspect();
    h.state_mut().run_window_command(&ctx, "window.set_catalog_mode", &json!({"window":0,"mode":"compact"})).unwrap();
    h.run();
    h.get_by_label("Expand Parts Catalog");
    assert_eq!(crate::window_commands::inspect_editor(h.state())["windows"][0]["catalog_mode"], "compact");
    let saved = h.state().parts.saved_state();
    for params in [json!({"window":0,"mode":"tiny"}), json!({"window":999,"mode":"expanded"}), json!({"window":0})] {
        assert!(h.state_mut().run_window_command(&ctx, "window.set_catalog_mode", &params).is_err());
        assert_eq!(h.state().parts.saved_state(), saved);
    }
    h.state_mut().run_window_command(&ctx, "window.set_catalog_mode", &json!({"window":0,"mode":"expanded"})).unwrap();
    h.run();
    h.get_by_label("Dock Parts Catalog");
    assert_eq!(h.state().session.inspect(), before);
}

/// Compact drags keep their dock open and commit once; an intervening edit invalidates only
/// that captured drag and cannot be overwritten by a later pointer release.
#[test]
fn compact_drags_place_once_and_reject_intervening_edits() {
    for stale in [false, true] {
        let mut h = editor(1400.);
        click(&mut h, "Dock Parts Catalog");
        let from = h.get_by_label("Capacitor").rect().center();
        let to = h.get_by_label("Schematic canvas").rect().center() + egui::vec2(100., 0.);
        h.drag_at(from);
        h.run();
        h.hover_at(from + egui::vec2(8., 0.));
        h.run();
        h.hover_at(to);
        h.run();
        if stale {
            let rev = h.state().session.revision();
            h.state_mut().session.apply(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":rev})).unwrap();
            h.run();
        }
        let revision = h.state().session.revision();
        h.drop_at(to);
        h.run();
        assert!(h.state().parts.open);
        assert_eq!(h.state().session.revision(), revision + u64::from(!stale));
        assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
        assert_eq!(h.state().session.project().unwrap().symbols()[0].definition.id, if stale { "resistor" } else { "capacitor" });
    }
}

/// Compact controls remain reachable as their dock follows the host, and floating a card
/// does not lose its dock/expand controls when the viewport shrinks.
#[test]
fn compact_docks_and_floats_stay_reachable_on_resize() {
    let mut h = editor(1400.);
    click(&mut h, "Dock Parts Catalog");
    for width in [700., 420., 1400.] {
        h.set_size(egui::vec2(width, 800.));
        h.run();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.));
        for label in ["Expand Parts Catalog", "Close Parts Catalog", "Place", "Library"] {
            assert!(screen.contains_rect(h.get_by_label(label).rect()), "{label} at width {width}");
        }
        assert!(h.state().parts.rect().intersect(h.state().browser.rect()).area() <= 0.);
    }
    click(&mut h, "Parts Catalog position");
    click(&mut h, "Float");
    h.set_size(egui::vec2(420., 800.));
    h.run();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420., 800.));
    assert!(screen.contains_rect(h.get_by_label("Expand Parts Catalog").rect()));
}

/// The explicit Repeat placement choice is shared across forms and persisted as presentation.
/// Once clears its tool after one placement; Repeat refreshes revision and preserves rotation.
#[test]
fn repeat_choice_is_shared_and_places_distinct_parts_until_escape() {
    for repeat in [false, true] {
        let mut h = editor(1400.);
        if repeat {
            click(&mut h, "Repeat placement");
        }
        click(&mut h, "Dock Parts Catalog");
        assert_eq!(h.state().parts.repeats(), repeat);
        let saved = h.state().parts.saved_state();
        assert_eq!(PartsBrowser::from_state(&saved).repeats(), repeat);
        click(&mut h, "Expand Parts Catalog");
        click(&mut h, "Capacitor");
        click(&mut h, "Place");
        h.key_press(Key::R);
        h.run();
        let rect = h.get_by_label("Schematic canvas").rect();
        crate::tests::click_canvas_at(&mut h, rect.center());
        assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
        if repeat {
            assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::PlaceSymbol { turns: 1, .. })));
            crate::tests::click_canvas_at(&mut h, rect.center() + egui::vec2(100., 80.));
            let project = h.state().session.project().unwrap();
            assert_eq!(project.symbols().len(), 2);
            assert_ne!(project.symbols()[0].id(), project.symbols()[1].id());
            assert_ne!(project.symbols()[0].position, project.symbols()[1].position);
            assert!(project.symbols().iter().all(|s| s.turns() == 1));
            h.key_press(Key::Escape);
            h.run();
        }
        assert!(h.state().views.active.schematic.tool.is_none());
        if repeat {
            click(&mut h, "Undo");
            assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
            click(&mut h, "Redo");
            assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
        }
        let count = h.state().session.project().unwrap().symbols().len();
        crate::tests::click_canvas_at(&mut h, rect.center() + egui::vec2(180., 80.));
        assert_eq!(h.state().session.project().unwrap().symbols().len(), count);
    }
}

/// New power anchors repeat with fresh IDs and the chosen name; moving an existing anchor
/// and stale placement previews cannot create extra instances.
#[test]
fn repeat_power_uses_fresh_ids_and_stops_after_intervening_edits() {
    let mut h = editor(1400.);
    click(&mut h, "Repeat placement");
    click(&mut h, "Cancel");
    h.key_press(Key::P);
    h.run();
    click(&mut h, "Place");
    h.key_press(Key::R);
    h.run();
    let center = h.get_by_label("Schematic canvas").rect().center();
    for offset in [egui::vec2(0., 0.), egui::vec2(100., 60.)] {
        crate::tests::click_canvas_at(&mut h, center + offset);
    }
    let project = h.state().session.project().unwrap();
    assert_eq!(project.net_labels().len(), 2);
    assert_ne!(project.net_labels()[0].id, project.net_labels()[1].id);
    assert!(project.net_labels().iter().all(|l| l.name == "GND" && l.rotation == 90));
    h.state_mut().run("project.rename", &json!({"name":"Intervening"}));
    h.run();
    let revision = h.state().session.revision();
    crate::tests::click_canvas_at(&mut h, center + egui::vec2(200., 100.));
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(h.state().session.project().unwrap().net_labels().len(), 2);
    assert!(h.state().views.active.schematic.tool.is_none());
}
