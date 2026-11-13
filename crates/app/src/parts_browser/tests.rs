use super::*;
use crate::{PcbApp, tests::click};
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

/// Open the real chooser in explicit one-shot mode for independent catalog interaction cases.
fn editor(width: f32) -> egui_kittest::Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.parts.set_repeat(false);
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
    h.get_by_label("Default");
    h.get_by_label("All libraries");
    let close = h.get_by_label("Close Parts Catalog").rect();
    let place = h.get_by_label("Place").rect();
    assert!(close.right() - place.left() < 400.);
    assert!(close.top() + 300. < place.bottom());
    click(&mut h, "Default");
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
    assert_eq!(h.state().parts.collection, Collection::Library("power".into()));
    assert!(h.query_by_label("Type").is_none());
    assert!(h.query_by_label("Preset").is_none());
    click(&mut h, "+3V3");
    assert_eq!(h.get_by_label("Value").value().as_deref(), Some("+3V3"));
    h.get_by_label("Value").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.run();
    assert!(h.get_by_label("Place").accesskit_node().is_disabled());
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().parts.open);
    assert!(h.state().views.active.schematic.tool.is_none());
    h.get_by_label("Value").click();
    h.get_by_label("Value").type_text("+3V3_ANALOG");
    h.run();
    click(&mut h, "Place");
    assert!(matches!(h.state().views.active.schematic.tool,Some(crate::Tool::PlaceSymbol{ref value,..}) if value=="+3V3_ANALOG"));
}

/// The flat-bar VCC is a distinct catalog choice but still names the VCC electrical net.
/// Both catalog presentations preserve its style/name through placement, editing and reopen.
#[test]
fn flat_bar_vcc_is_an_additional_power_choice() {
    use pcbcraft_engine::persistence::{decode_documents, encode_documents};
    for (width, compact) in [(1400., false), (700., false), (1400., true)] {
        let mut h = editor(width);
        click(&mut h, "Cancel");
        if compact {
            h.state_mut().parts.mode = Mode::Compact;
        }
        h.key_press(Key::P);
        h.run();
        let before = h.state().session.inspect();
        h.get_by_label("VCC (bar)").scroll_to_me();
        h.run();
        click(&mut h, "VCC (bar)");
        assert_eq!(h.get_by_label("Value").value().as_deref(), Some("VCC"), "width={width}, compact={compact}");
        assert_eq!(h.state().parts.selected, Some(super::Key::Symbol("power".into(), "vcc-bar".into(), 1)));
        h.get_by_label("VCC").scroll_to_me();
        h.run();
        click(&mut h, "VCC");
        assert_eq!(h.state().parts.selected, Some(super::Key::Symbol("power".into(), "vcc".into(), 1)));
        h.get_by_label("VCC (bar)").scroll_to_me();
        h.run();
        click(&mut h, "VCC (bar)");
        assert_eq!(h.state().session.inspect(), before);
        click(&mut h, "Place");
        assert!(
            matches!(&h.state().views.active.schematic.tool,Some(crate::Tool::PlaceSymbol {definition,value,..}) if definition.id=="vcc-bar" && value=="VCC")
        );
        h.get_by_label("Schematic canvas").click();
        h.run();
        let project = h.state().session.project().unwrap();
        assert_eq!((&*project.symbols()[0].definition.id, project.component(&project.symbols()[0]).unwrap().value()), ("vcc-bar", "VCC"));
        let documents = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
        let ctx = h.ctx.clone();
        h.state_mut().parts.close(&ctx);
        h.run();
        h.key_press(Key::E);
        h.run();
        assert!(matches!(h.state().dialog, crate::Dialog::EditFields(_)));
        assert_eq!(h.get_by_label("Value").value().as_deref(), Some("VCC"));
        h.get_by_label("Value").click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.get_by_label("Value").type_text("VCC_ANALOG");
        h.run();
        click(&mut h, "OK");
        let project = h.state().session.project().unwrap();
        assert_eq!(project.component(&project.symbols()[0]).unwrap().value(), "VCC_ANALOG");
        click(&mut h, "Undo");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), documents);
    }
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

/// Library management stays available when a search has no matches and placement is disabled.
#[test]
fn catalog_reaches_existing_library_manager() {
    let mut h = editor(1400.);
    h.get_by_label("Search parts").type_text("missing-catalog-item");
    h.run();
    assert!(h.get_by_label("Place").accesskit_node().is_disabled());
    click(&mut h, "Manage libraries…");
    assert!(!h.state().parts.open);
    assert_eq!(h.state().workspace_key(), "library");
    h.get_by_label("Built-in");
}

/// Catalog selection does not use display text as identity, and source filters use library IDs.
#[test]
fn library_search_preserves_native_identity() {
    assert_eq!(matches_sources("CAP", &library::sources::Catalog::bundled())[0].id, "capacitor");
    assert!(matches_sources("zzz", &library::sources::Catalog::bundled()).is_empty());
    assert_eq!(matches_sources("j", &library::sources::Catalog::bundled()).len(), 2);
    let catalog = library::sources::Catalog::bundled();
    assert_eq!(entries("", &catalog).iter().filter(|e| e.in_collection(&Collection::Library("power".into()))).count(), 9);
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
        h.get_by_label("Workshop resistor");
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
    click(&mut h, "Default");
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
    click(&mut h, "Value");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Value").type_text("V_ANALOG");
    h.run();
    click(&mut h, "Dock Parts Catalog");
    assert_eq!(h.get_by_label("Value").value().as_deref(), Some("V_ANALOG"));
    assert!(h.state().parts.rect().contains_rect(h.get_by_label("+3V3").rect()));
    click(&mut h, "Expand Parts Catalog");
    h.get_by_label("Preview +3V3");
    assert_eq!(h.get_by_label("Value").value().as_deref(), Some("V_ANALOG"));
    click(&mut h, "Dock Parts Catalog");
    click(&mut h, "Place");
    assert!(h.state().parts.open);
    assert!(matches!(h.state().views.active.schematic.tool, Some(crate::Tool::PlaceSymbol{ref value,..}) if value=="V_ANALOG"));
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
    assert!(PartsBrowser::from_state(&previous).repeats());
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

/// Power tiles share component dragging in both catalog presentations. A drop commits one
/// marker at the snapped pointer, retains its custom name/style and arms repeat with a fresh ID.
#[test]
fn power_catalog_drags_preview_place_and_repeat_in_both_presentations() {
    for compact in [false, true] {
        for (tile, name, style) in [("GND", "GND", "gnd"), ("VCC (bar)", "VCC_ANALOG", "vcc-bar")] {
            let mut h = editor(1400.);
            h.state_mut().browser.open = false;
            h.run();
            click(&mut h, "Cancel");
            h.key_press(Key::P);
            h.run();
            click(&mut h, "List");
            if compact {
                click(&mut h, "Dock Parts Catalog");
            }
            h.get_by_label(tile).scroll_to_me();
            h.run();
            click(&mut h, tile);
            if name != "GND" {
                h.get_by_label("Value").click();
                h.run();
                h.key_press_modifiers(Modifiers::COMMAND, Key::A);
                h.get_by_label("Value").type_text(name);
                h.run();
            }
            click(&mut h, "Repeat placement");
            let before = h.state().session.snapshot().unwrap();
            let from = h.get_by_label(tile).rect().center();
            let to = h.get_by_label("Schematic canvas").rect().center() + egui::vec2(150., 60.);
            h.drag_at(from);
            h.run();
            h.hover_at(from + egui::vec2(8., 0.));
            h.run();
            assert!(egui::DragAndDrop::payload::<crate::PartDrag>(&h.ctx).is_some(), "power drag starts: compact={compact} tile={tile}");
            h.hover_at(to);
            h.run();
            assert_eq!(h.state().parts.open, compact);
            assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            let drag = egui::DragAndDrop::payload::<crate::PartDrag>(&h.ctx).expect("power payload follows the pointer onto the canvas");
            assert!(matches!(&drag.placement, Placement { definition,value, .. } if value == name && definition.id == style));
            let canvas = h.get_by_label("Schematic canvas").rect();
            let point = h.state().views.active.camera.snap(to, canvas);
            h.drop_at(to);
            h.run();
            assert_eq!(h.state().session.revision(), drag.source.revision + 1);
            let project = h.state().session.project().unwrap();
            let symbol = project.symbols()[0].clone();
            assert_eq!((symbol.position, project.component(&symbol).unwrap().value(), symbol.definition.id.as_str()), (point, name, style));
            assert!(matches!(&h.state().views.active.schematic.tool, Some(crate::Tool::PlaceSymbol { .. })));
            crate::tests::click_canvas_at(&mut h, to + egui::vec2(100., 60.));
            let after = h.state().session.snapshot().unwrap();
            assert_eq!(after.symbols().len(), 2);
            assert_ne!(after.symbols()[0].id(), after.symbols()[1].id());
            assert!(after.net_labels().is_empty());
            h.key_press(Key::Escape);
            h.run();
            click(&mut h, "Undo");
            assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
            click(&mut h, "Undo");
            crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
            click(&mut h, "Redo");
            click(&mut h, "Redo");
            crate::tests::assert_same_content(&after, &h.state().session.snapshot().unwrap());
        }
    }
}

/// Dragging another supply uses that tile's own default name. One-shot, invalid names,
/// stale revisions, Escape and release over the catalog never create unintended markers.
#[test]
fn power_drags_use_their_own_names_and_reject_invalid_or_cancelled_drops() {
    for mode in ["place", "invalid", "stale", "escape", "panel"] {
        let mut h = editor(1400.);
        h.state_mut().browser.open = false;
        h.run();
        click(&mut h, "Cancel");
        h.key_press(Key::P);
        h.run();
        click(&mut h, "List");
        click(&mut h, "Dock Parts Catalog");
        if mode == "invalid" {
            click(&mut h, "+3V3");
            h.get_by_label("Value").click();
            h.run();
            h.key_press_modifiers(Modifiers::COMMAND, Key::A);
            h.key_press(Key::Backspace);
            h.run();
        }
        let from = h.get_by_label("+3V3").rect().center();
        let to = h.get_by_label("Schematic canvas").rect().center() + egui::vec2(150., 60.);
        h.drag_at(from);
        h.run();
        h.hover_at(from + egui::vec2(8., 0.));
        h.run();
        h.hover_at(to);
        h.run();
        if mode == "stale" {
            h.state_mut().run("project.rename", &json!({"name":"Intervening"}));
            h.run();
        } else if mode == "escape" {
            h.key_press(Key::Escape);
            h.run();
        }
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        h.drop_at(if mode == "panel" { from } else { to });
        h.run();
        let placed = mode == "place";
        assert_eq!(h.state().session.revision(), revision + u64::from(placed), "{mode}");
        assert!(h.state().views.active.schematic.tool.is_none(), "{mode}");
        if placed {
            assert_eq!(h.state().session.project().unwrap().components().next().unwrap().value(), "+3V3");
        } else {
            assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "{mode}");
        }
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
    click(&mut h, "GND");
    click(&mut h, "Place");
    h.key_press(Key::R);
    h.run();
    let center = h.get_by_label("Schematic canvas").rect().center();
    for offset in [egui::vec2(0., 0.), egui::vec2(100., 60.)] {
        crate::tests::click_canvas_at(&mut h, center + offset);
    }
    let project = h.state().session.project().unwrap();
    assert_eq!(project.symbols().len(), 2);
    assert_ne!(project.symbols()[0].id(), project.symbols()[1].id());
    assert!(project.symbols().iter().all(|s| project.component(s).unwrap().value() == "GND" && s.turns() == 1));
    h.state_mut().run("project.rename", &json!({"name":"Intervening"}));
    h.run();
    let revision = h.state().session.revision();
    crate::tests::click_canvas_at(&mut h, center + egui::vec2(200., 100.));
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
    assert!(h.state().views.active.schematic.tool.is_none());
}

/// The same live panel command can close the expanded catalog it opened, while engineering
/// commands remain refused by name and the shared project data/history remain untouched.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_panel_commands_open_and_close_modal_catalog_without_switching_modes() {
    use crate::live_control::LiveEditor;
    use pcbcraft_mcp::live::call_editor;
    let mut h = editor(1400.);
    click(&mut h, "Cancel");
    let ctx = h.ctx.clone();
    let before = h.state().session.inspect();
    for open in [true, false] {
        let result = call_editor(
            &mut LiveEditor { app: h.state_mut(), ctx: &ctx },
            &json!({"method":"engine.execute","params":{"command":"window.set_panel","params":{"window":0,"panel":"parts","open":open}}}),
        );
        assert!(result.is_ok(), "{result:?}");
        h.run();
        assert_eq!(h.state().parts.is_modal(), open);
        if open {
            let revision = h.state().session.revision();
            let refused = call_editor(
                &mut LiveEditor { app: h.state_mut(), ctx: &ctx },
                &json!({"method":"engine.execute","params":{"command":pcbcraft_engine::PLACE_RESISTOR,"params":{"x_nm":0,"y_nm":0,"expected_revision":revision}}}),
            );
            assert!(format!("{refused:?}").contains("Parts Catalog"));
        }
        assert_eq!(h.state().session.inspect(), before);
    }
}

/// Loading an equivalent bundled folder retains one library filter and one row per identity.
#[test]
fn equivalent_source_registrations_do_not_duplicate_catalog_libraries() {
    let mut h = editor(1400.);
    let (_, docs) = library::bundled_documents().find(|(id, _)| *id == "starter").unwrap();
    h.state_mut().session.execute(library::sources::LOAD, &json!({"source":"folder-copy","documents":docs})).unwrap();
    h.run();
    assert_eq!(collections(&h.state().session.libraries().inspect()).len(), 3);
    assert_eq!(entries("", h.state().session.libraries()).len(), 15);
    h.get_by_label("Default");
    click(&mut h, "Default");
    h.get_by_label("Capacitor");
}

/// Catalog placement and property edits preserve the central native part/MPN relation through the GUI path.
#[test]
fn native_part_catalog_and_requirements_use_shared_transactions() {
    let mut h = editor(1400.);
    click(&mut h, "Cancel");
    let documents = native_documents();
    h.state_mut().session.execute(library::sources::LOAD, &json!({"source":"native-parts","documents":documents})).unwrap();
    h.key_press(Key::A);
    h.run();
    click(&mut h, "Native Parts Demo");
    click(&mut h, "Generic resistor");
    h.get_by_role_and_label(egui::Role::ComboBox, "Manufacturer part").click();
    h.run();
    click(&mut h, "Illustrative Manufacturer · TEST-R100M-0603");
    assert_eq!(h.state().parts.value, "100 mΩ");
    click(&mut h, "Place");
    click(&mut h, "Schematic canvas");
    let project = h.state().session.project().unwrap();
    let part = project.components().next().unwrap();
    assert!(part.part.as_ref().unwrap().manufacturer.is_some());
    assert_eq!(part.value(), "0.1 Ohm");
    assert_eq!(project.footprint_assignments().len(), 1);
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    assert!(matches!(h.state().dialog, crate::Dialog::NativePart(_)));
    h.get_by_role_and_label(egui::Role::ComboBox, "Tolerance").click();
    h.run();
    click(&mut h, "At most");
    h.get_by_role_and_label(egui::Role::TextInput, "Tolerance value").click();
    h.run();
    h.get_by_role_and_label(egui::Role::TextInput, "Tolerance value").type_text("0.005");
    h.run();
    assert_eq!(h.get_by_role_and_label(egui::Role::TextInput, "Tolerance value").value().as_deref(), Some("0.005"));
    click(&mut h, "Apply");
    assert!(matches!(h.state().dialog, crate::Dialog::None), "{:?}", h.state().dialog);
    let selection = h.state().session.project().unwrap().components().next().unwrap().part.as_ref().unwrap();
    assert_eq!(selection.compatibility()["status"], "does_not_meet_requirements");
    assert!(selection.manufacturer.is_some());
    let component = h.state().session.project().unwrap().components().next().unwrap().id();
    h.state_mut().begin_footprint_assignment(Some(component));
    h.run();
    assert!(matches!(h.state().dialog, crate::Dialog::NativePart(_)));
    h.get_by_role_and_label(egui::Role::ComboBox, "Resistance").click();
    h.run();
    click(&mut h, "One of");
    click(&mut h, "Add allowed value");
    for width in [1400., 700., 420.] {
        h.set_size(egui::vec2(width, 1000.));
        h.run();
        h.get_by_role_and_label(egui::Role::Button, "Remove Resistance value 2").scroll_to_me();
        h.run();
        let button = h.get_by_role_and_label(egui::Role::Button, "Remove Resistance value 2").rect();
        let input = h.get_by_role_and_label(egui::Role::TextInput, "Resistance value 2").rect();
        assert!(button.right() <= width && input.right() <= button.left(), "one-of row overflows at {width}: {input:?}, {button:?}");
    }
    h.get_by_role_and_label(egui::Role::Button, "Remove Resistance value 2").click();
    h.run();
    assert!(h.query_by_role_and_label(egui::Role::TextInput, "Resistance value 2").is_none());
    click(&mut h, "Cancel");
}

/// Original part fixtures are shared across catalog identity and native-selection workflows.
fn native_documents() -> library::sources::Documents {
    [
        ("library.json", include_str!("../../../../examples/native-parts/library.json")),
        ("symbols/resistor.json", include_str!("../../../../examples/native-parts/symbols/resistor.json")),
        ("footprints/resistor-0603.json", include_str!("../../../../examples/native-parts/footprints/resistor-0603.json")),
        ("categories/resistor.json", include_str!("../../../../examples/native-parts/categories/resistor.json")),
        ("parts/resistor.json", include_str!("../../../../examples/native-parts/parts/resistor.json")),
        ("manufacturer-parts/test.json", include_str!("../../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json")),
    ]
    .into_iter()
    .map(|(path, text)| (path.into(), text.into()))
    .collect()
}

/// A deleted typed part retains its formatted value in both the real Browser row
/// and selected-footprint Properties without presentation changing the document.
#[test]
fn retained_part_quantity_is_shared_by_board_properties_and_browser() {
    use pcbcraft_engine::{part_instances::PLACE_PART, persistence::encode_documents};
    let mut h = editor(1400.);
    click(&mut h, "Cancel");
    h.state_mut().run("project.new", &json!({"create_board":true}));
    let mut documents = native_documents();
    let mut part: serde_json::Value = serde_json::from_str(&documents["parts/resistor.json"]).unwrap();
    part["fixed_parameters"] = json!({"pcbcraft:resistance":{"type":"quantity","value":"0.1","unit":"Ohm"}});
    documents.insert("parts/resistor.json".into(), part.to_string());
    h.state_mut().session.execute(library::sources::LOAD, &json!({"source":"native","documents":documents})).unwrap();
    let reference = library::parts::Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
    let resolved = h.state().session.libraries().resolve_part(&reference).unwrap();
    let digest = library::sources::part_digest(resolved);
    let edit = |h: &mut egui_kittest::Harness<'_, PcbApp>, command: &str, mut params: serde_json::Value| {
        params["expected_revision"] = json!(h.state().session.revision());
        h.state_mut().run(command, &params);
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        h.run();
    };
    edit(
        &mut h,
        PLACE_PART,
        json!({"library":"parts-demo","definition":"resistor","definition_revision":1,
        "definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    let id = h.state().session.project().unwrap().components().next().unwrap().id();
    edit(&mut h, pcbcraft_engine::PLACE_ASSIGNED_FOOTPRINT, json!({"component":id,"x_nm":0,"y_nm":0}));
    edit(&mut h, pcbcraft_engine::INITIALIZE_UPDATE_BASELINE, json!({}));
    let symbol = h.state().session.project().unwrap().symbols()[0].id();
    edit(&mut h, pcbcraft_engine::DELETE_SYMBOL, json!({"symbol":symbol}));
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    click(&mut h, "Footprint R1  100 mΩ");
    let snapshot = h.state().session.snapshot().unwrap();
    let saved = encode_documents(snapshot.clone()).unwrap();
    let revision = h.state().session.revision();
    for width in [1400., 900.] {
        h.set_size(egui::vec2(width, 900.));
        h.run();
        h.get_by_label("Footprint R1  100 mΩ");
        assert!(
            h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text)
            if text.galley.job.text == "100 mΩ")),
            "Properties must show the same SI value as the Browser"
        );
        assert_eq!(h.state().session.revision(), revision);
        assert!(std::sync::Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
    }
}

/// Equally named parts can share a drawing while keeping distinct visible values and identities.
#[test]
fn native_catalog_disambiguates_parts_sharing_a_symbol() {
    let mut h = editor(1400.);
    click(&mut h, "Cancel");
    let mut documents = native_documents();
    let mut part: serde_json::Value = serde_json::from_str(&documents["parts/resistor.json"]).unwrap();
    part["id"] = json!("fixed-resistor");
    part["fixed_parameters"] = json!({"pcbcraft:resistance":{"type":"quantity","value":"1000","unit":"Ohm"}});
    documents.insert("parts/fixed.json".into(), part.to_string());
    h.state_mut().session.execute(library::sources::LOAD, &json!({"source":"native-parts","documents":documents})).unwrap();
    h.key_press(Key::A);
    h.run();
    click(&mut h, "Native Parts Demo");
    h.get_by_label("Generic resistor · parts-demo/resistor r1");
    h.get_by_label("Generic resistor · parts-demo/fixed-resistor r1");
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text == "1 kΩ")));
    click(&mut h, "Generic resistor · parts-demo/fixed-resistor r1");
    assert_eq!(h.state().parts.value, "1 kΩ");
    click(&mut h, "Place");
    click(&mut h, "Schematic canvas");
    assert_eq!(h.state().session.project().unwrap().components().next().unwrap().value(), "1000 Ohm");
}

/// Switching from a generic symbol to a native part opens native package selection before assignment.
#[test]
fn assignment_component_browser_routes_native_parts() {
    let mut h = editor(1400.);
    click(&mut h, "Cancel");
    h.state_mut().run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1}));
    h.state_mut().session.execute(library::sources::LOAD, &json!({"source":"native-parts","documents":native_documents()})).unwrap();
    h.key_press(Key::A);
    h.run();
    click(&mut h, "Native Parts Demo");
    click(&mut h, "Generic resistor");
    click(&mut h, "Place");
    click(&mut h, "Schematic canvas");
    let reference = h.state().session.project().unwrap().components().find(|part| part.part.is_some()).unwrap().reference().to_owned();
    let value = h.state().session.project().unwrap().components().find(|part| part.part.is_some()).unwrap().value().to_owned();
    h.state_mut().begin_footprint_assignment(None);
    h.run();
    assert!(matches!(h.state().dialog, crate::Dialog::Footprint(_)));
    click(&mut h, &format!("{reference} · {value}"));
    assert!(matches!(h.state().dialog, crate::Dialog::NativePart(_)));
    h.get_by_role_and_label(egui::Role::ComboBox, "Package");
    click(&mut h, "Cancel");
}
