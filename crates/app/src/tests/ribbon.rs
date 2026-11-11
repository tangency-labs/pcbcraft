//! Acceptance for the ported toolbar's command routing and compact presentation.
use super::*;

/// Short and long labels retain matching targets, including disabled panel commands and
/// controls scrolled into view on compact windows in either workspace.
#[test]
fn view_ribbon_buttons_have_consistent_sizes() {
    for workspace in [Workspace::Schematic, Workspace::Board] {
        for width in [1400.0, 420.0] {
            let mut app = PcbApp::default();
            app.start_new_project_in(workspace);
            let mut h = Harness::builder().with_size(egui::vec2(width, 850.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            click(&mut h, "View");
            let expected = h.get_by_label("Show grid").rect().size();
            for name in ["Toggle Browser", "Open Parts Catalog", "Toggle Net Inspector", "Toggle Properties", "Toggle Selection filters"] {
                h.get_by_label(name).scroll_to_me();
                h.run();
                assert_eq!(h.get_by_label(name).rect().size(), expected, "{workspace:?} {width}: {name}");
            }
            let fit = if workspace == Workspace::Schematic { "Fit schematic" } else { "Fit board" };
            h.get_by_label(fit).scroll_to_me();
            h.run();
            let button = h.get_by_label(fit).rect();
            assert_eq!(button.size(), expected);
            let center = if workspace == Workspace::Board {
                (button.center().x + h.get_by_label("View the board from below").rect().center().x) / 2.0
            } else {
                button.center().x
            };
            assert!((center - h.get_by_label("Navigate menu").rect().center().x).abs() <= 1.0);
        }
    }
}

/// Schematic and PCB grid controls work in both ribbon layouts without changing project history,
/// snap spacing, opacity or the PCB's chosen line style. Menu actions share the same visibility.
#[test]
fn ribbon_grid_visibility_is_independent_between_workspaces() {
    for width in [1400.0, 420.0] {
        let mut app = PcbApp::default();
        app.start_new_project_in(Workspace::Board);
        app.preferences.grid_opacity_percent = 35;
        app.preferences.board_grid_style = grid::Style::Lines;
        let mut h = Harness::builder().with_size(egui::vec2(width, 850.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        choose(&mut h, "Choose workspace", "Schematic");
        click(&mut h, "View");
        let before = h.state().session.snapshot().unwrap();
        let spacing = h.state().views.active.camera.snap_nm();
        click(&mut h, "Show grid");
        assert!(!h.state().preferences.grid_visible(false));
        assert!(h.state().preferences.grid_visible(true));
        let canvas = h.get_by_label("Schematic canvas").rect();
        assert!(super::split_editors::painted_grid_dots(&h, canvas).is_empty());
        click(&mut h, "Show grid");
        assert!(h.state().preferences.grid_visible(false));
        assert!(!super::split_editors::painted_grid_dots(&h, canvas).is_empty());
        choose(&mut h, "Display menu", "Grid");
        h.key_press(Key::Escape);
        h.run();
        assert!(!h.state().preferences.grid_visible(false));
        choose(&mut h, "Choose workspace", "PCB layout");
        click(&mut h, "View");
        click(&mut h, "Show grid");
        assert!(!h.state().preferences.grid_visible(true));
        assert!(!h.state().preferences.grid_visible(false));
        click(&mut h, "Show grid");
        assert!(h.state().preferences.grid_visible(true));
        assert!(!h.state().preferences.grid_visible(false));
        assert_eq!(h.state().preferences.grid_opacity_percent, 35);
        assert_eq!(h.state().preferences.board_grid_style, grid::Style::Lines);
        assert_eq!(h.state().views.active.camera.snap_nm(), spacing);
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        let restored = preferences::Preferences::parse(&h.state().preferences.to_text());
        assert!(!restored.grid_visible(false) && restored.grid_visible(true));
    }
}

/// Both promoted buttons and caption menus reach the same sheet/label tools without
/// editing the document until the existing placement form is committed.
#[test]
fn ribbon_reaches_sheet_and_hierarchical_label_workflows_at_both_widths() {
    for width in [1400., 400.] {
        let mut h = harness();
        h.set_size(egui::vec2(width, 800.));
        h.run();
        new_project(&mut h);
        let original = h.state().session.inspect();
        let revision = h.state().session.revision();
        for (tab, slot) in [("Analyze", 2), ("Libraries", 3), ("Sheets", 4), ("View", 1), ("Capture", 0)] {
            click_tab(&mut h, tab);
            assert_eq!(h.state().ribbon_tab, slot);
        }
        if width < 1000. {
            click(&mut h, "Place menu");
            assert!(!h.get_by_label("Ground").accesskit_node().is_disabled());
            h.key_press(Key::Escape);
            h.run();
        } else {
            assert!(!h.get_by_label("Place ground").accesskit_node().is_disabled());
        }
        click_tab(&mut h, "Sheets");
        // The scrolled button and its caption-menu entry must target the same view.
        h.get_by_label("Place hierarchical sheet (S)").scroll_to_me();
        h.run();
        click(&mut h, "Place hierarchical sheet (S)");
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Hierarchy { kind: crate::hierarchy::Kind::Sheet, .. })));
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().views.active.schematic.tool.is_none());
        h.get_by_label("Sheets menu").scroll_to_me();
        h.run();
        click(&mut h, "Sheets menu");
        click(&mut h, "Hierarchical label");
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Hierarchy { kind: crate::hierarchy::Kind::Label, .. })));
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().views.active.schematic.tool.is_none());
        assert_eq!(h.state().session.inspect(), original);
        assert_eq!(h.state().session.revision(), revision);

        click_tab(&mut h, "Sheets");
        click(&mut h, "Sheets menu");
        click(&mut h, "New sheet");
        assert!(matches!(h.state().dialog, Dialog::Sheet(_)));
        h.get_by_label("Name").type_text("Power");
        h.run();
        click(&mut h, "OK");
        assert_eq!(h.state().session.project().unwrap().sheets().count(), 2);
        click(&mut h, "Undo");
        assert_eq!(h.state().session.project().unwrap().sheets().count(), 1);
        click(&mut h, "Redo");
        assert_eq!(h.state().session.project().unwrap().sheets().count(), 2);
    }
}

/// Longer tab lists must leave every tab clickable at the app's compact window widths.
#[test]
fn all_five_tabs_fit_the_compact_header() {
    let mut h = harness();
    new_project(&mut h);
    for width in [320., 400., 520., 700.] {
        h.set_size(egui::vec2(width, 800.));
        h.run();
        for tab in ["Capture", "Analyze", "Libraries", "Sheets", "View"] {
            let rect = h.get_by_role_and_label(egui::Role::Tab, tab).rect();
            assert!(rect.left() >= 0. && rect.right() <= width, "{width}: {tab} {rect:?}");
            click_tab(&mut h, tab);
        }
        click(&mut h, "Capture");
    }
}

/// Move picks up the selected symbol like M and remains cancelable without editing the session.
#[test]
fn move_button_uses_the_symbol_move_workflow() {
    let mut h = harness();
    wiring_project(&mut h);
    let symbol = h.state().session.project().unwrap().symbols()[0].id();
    h.state_mut().select(Selection::one_symbol(symbol.clone()));
    h.run();
    let before = h.state().session.inspect();
    click(&mut h, "Move selected object (M)");
    assert!(matches!(&h.state().views.active.schematic.tool, Some(Tool::Move { symbol: moved, disconnect:true, .. }) if moved.id()==symbol));
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().views.active.schematic.tool.is_none());
    assert_eq!(h.state().session.inspect(), before);
}

/// A ribbon tool scrolled under the ribbon's left edge is still inside the window, but hidden.
/// The ribbon reports its visible viewport, so the test `click` helper scrolls the tool back
/// into view before clicking instead of clicking the hidden spot. Size the window from the
/// measured scroll displacement so adding another ribbon tool preserves this clipping case.
#[test]
fn clicks_reach_a_tool_scrolled_under_the_ribbon_edge() {
    const RECTANGLE: &str = "Draw rectangle on active layer";
    let mut h = harness();
    h.set_size(egui::vec2(1100., 700.));
    resistor_with_board_at_origin(&mut h);
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    click(&mut h, "Draw");
    h.set_size(egui::vec2(320., 850.));
    h.run();
    let selector = h.get_by_label("Active layer").rect();
    click(&mut h, RECTANGLE);
    assert_eq!(h.get_by_label("Active layer").rect(), selector);
    assert_eq!(h.state().views.active.board.drawing.tool, Some(crate::board_drawing::Tool::Rectangle));
}

/// PCB tasks have distinct tabs; planned tools remain disabled and setup opens the
/// existing board form. Tab switches and menu inspection never edit project history.
#[test]
fn pcb_tasks_are_grouped_on_separate_tabs() {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    resistor_with_board_at_origin(&mut h);
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let before = h.state().session.inspect();
    for group in ["Place", "Route", "Clipboard", "Modify", "Board"] {
        h.get_by_label(&format!("{group} menu"));
    }
    for tool in ["Assign and place component footprints", "Update PCB from schematic", "Board setup"] {
        assert!(!h.get_by_label(tool).accesskit_node().is_disabled());
    }
    for tool in ["Run design rules check", "Measure distance", "Generate fabrication outputs", "View the board from below"] {
        assert!(h.query_by_label(tool).is_none(), "{tool} should have its own task tab");
    }
    click(&mut h, "Board setup");
    assert!(!matches!(h.state().dialog, Dialog::None));
    click(&mut h, "Cancel");
    click(&mut h, "Layout");
    h.get_by_label("Place vias");
    h.get_by_label("Route tracks on the active copper layer");
    assert!(h.query_by_role_and_label(egui::Role::Tab, "Route").is_none());
    assert!(h.query_by_label("Align menu").is_none());
    h.get_by_label("Active layer");
    click(&mut h, "Route menu");
    for tool in ["Zone", "Pair", "Tune", "Refill zones"] {
        assert!(h.get_by_label(tool).accesskit_node().is_disabled());
    }
    h.key_press(Key::Escape);
    h.run();
    click(&mut h, "Analyze");
    h.get_by_label("Run design rules check");
    h.get_by_label("Measure distance");
    h.get_by_label("Reveal highlighted counterpart");
    click(&mut h, "Checks menu");
    assert!(h.get_by_label("Rules").accesskit_node().is_disabled());
    h.key_press(Key::Escape);
    h.run();
    click(&mut h, "Manufacture");
    h.get_by_label("Generate fabrication outputs");
    click(&mut h, "View");
    h.get_by_label("View the board from below");
    assert_eq!(h.state().session.inspect(), before);
}

/// Six PCB tabs remain reachable at compact widths; switching tasks keeps the authored
/// board untouched and selecting a task restores its actual control surface.
#[test]
fn all_pcb_tabs_fit_compact_headers() {
    let mut app = PcbApp::default();
    app.start_new_project_in(Workspace::Board);
    let mut h = Harness::builder().with_size(egui::vec2(1400., 850.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    for width in [320., 400., 520., 700., 1400.] {
        h.set_size(egui::vec2(width, 850.));
        h.run();
        for tab in ["Layout", "Draw", "Analyze", "Manufacture", "Libraries", "View"] {
            let rect = h.get_by_role_and_label(egui::Role::Tab, tab).rect();
            assert!(rect.left() >= 0. && rect.right() <= width, "{width}: {tab} {rect:?}");
            click_tab(&mut h, tab);
        }
        click(&mut h, "Layout");
        h.get_by_label("Place vias");
    }
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Wide windows leave spare room outside the command groups. Board stays beside Modify
/// instead of centering its two tools in all the space before the pinned editing fields.
#[test]
fn board_group_stays_beside_modify_when_the_window_widens() {
    let mut app = PcbApp::default();
    app.start_new_project_in(Workspace::Board);
    let mut h = Harness::builder().with_size(egui::vec2(1700., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let mut initial = None;
    for width in [1700., 2100., 2600.] {
        h.set_size(egui::vec2(width, 900.));
        h.run();
        let tools = h.get_by_label("Update PCB from schematic").rect().union(h.get_by_label("Board setup").rect());
        let previous = h.get_by_label("Modify menu").rect();
        assert!(tools.left() - previous.right() <= 64., "{width}: Board must stay adjacent to Modify");
        if let Some(initial) = initial {
            assert_eq!(tools, initial, "{width}: wider windows must not move the Board buttons");
        } else {
            initial = Some(tools);
        }
    }
}

/// Layer and physical-size controls stay visible, in the same place and with the same
/// widget identities on each PCB tab; changing tabs cannot clear their values.
#[test]
fn pcb_editing_fields_are_persistent_outside_the_commands() {
    let mut app = PcbApp::default();
    app.start_new_project_in(Workspace::Board);
    let mut h = Harness::builder().with_size(egui::vec2(1400., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.ctx.options_mut(|options| options.warn_on_id_clash = true);
    h.run();
    for width in [1400., 900., 760., 520., 400., 320.] {
        h.set_size(egui::vec2(width, 900.));
        h.run();
        let labels = ["Active layer", "Track / line width", "Via pad / drill"];
        let bounds: Vec<_> = labels.iter().map(|name| h.get_by_label(name).rect()).collect();
        let identities: Vec<_> = labels.iter().map(|name| h.get_by_label(name).accesskit_node().id()).collect();
        for tab in ["Layout", "Draw", "Analyze", "Manufacture", "Libraries", "View"] {
            click_tab(&mut h, tab);
            let clashes: Vec<_> = h
                .output()
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text.contains("use of widget ID") => Some((text.galley.job.text.clone(), text.pos)),
                    _ => None,
                })
                .collect();
            assert!(clashes.is_empty(), "{width} {tab}: editing fields must not produce widget-ID clash warnings: {clashes:?}");
            let section = h
                .output()
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "Layer & sizes" => Some(text.visual_bounding_rect()),
                    _ => None,
                })
                .unwrap();
            let fields = labels.iter().map(|name| h.get_by_label(name).rect()).reduce(egui::Rect::union).unwrap();
            assert!((section.center().x - fields.center().x).abs() <= 1.0, "{width} {tab}: section caption {section:?}, fields {fields:?}");
            assert!(section.bottom() <= h.get_by_label("PCB canvas").rect().top(), "{width} {tab}: section caption overlaps canvas");
            assert!(h.query_by_role_and_label(egui::Role::Button, "Layer & sizes").is_none());
            if width == 1400. && tab == "Layout" {
                let caption_y = |label: &str| {
                    h.output()
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.job.text == label => Some(text.pos.y),
                            _ => None,
                        })
                        .unwrap()
                };
                assert_eq!(caption_y("Layer & sizes"), caption_y("Board"), "section labels must share the toolkit's baseline");
            }
            for (index, name) in labels.iter().enumerate() {
                let node = h.get_by_label(name);
                let rect = node.rect();
                let caption = h
                    .output()
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == *name => Some(text.visual_bounding_rect()),
                        _ => None,
                    })
                    .unwrap();
                assert!((caption.center().x - rect.center().x).abs() <= 1.0, "{width} {tab}: {name} caption {caption:?}, control {rect:?}");
                if caption.top() > rect.top() {
                    assert!(
                        caption.top() - rect.bottom() <= 10.0,
                        "{width} {tab}: {name} caption {caption:?}, dropdown {rect:?} has too much spacing"
                    );
                }

                assert_eq!(rect, bounds[index], "{width} {tab}: {name}");
                assert_eq!(node.accesskit_node().id(), identities[index], "{width} {tab}: {name}");
                assert!(rect.left() >= 0. && rect.right() <= width, "{width} {tab}: {name} {rect:?}");
                assert!(
                    rect.bottom() <= h.get_by_label("PCB canvas").rect().top(),
                    "{width} {tab}: field overlaps canvas at {}: {name} {rect:?}",
                    h.get_by_label("PCB canvas").rect().top()
                );
            }
        }
    }
}

/// Custom widths accept display units and remain reusable after local settings are restored;
/// picking or adding a width cannot edit project geometry or history.
#[test]
fn pcb_stroke_width_accepts_typed_values() {
    use pcbcraft_chrome::units::Unit;
    for width in [1400., 760.] {
        for (unit, text, expected) in [(Unit::Mm, "0.4", 400_000), (Unit::Mil, "20", 508_000)] {
            let mut app = PcbApp::default();
            app.start_new_project_in(Workspace::Board);
            app.preferences.units = unit;
            let mut h = Harness::builder().with_size(egui::vec2(width, 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let before = h.state().session.snapshot().unwrap();
            click(&mut h, "Track / line width");
            click(&mut h, "Custom…");
            click(&mut h, "Width");
            h.key_press_modifiers(Modifiers::COMMAND, Key::A);
            h.get_by_label("Width").type_text(text);
            h.run();
            click(&mut h, "Use");
            assert_eq!(h.state().views.active.board.tracks.width_nm, expected, "{width} {unit:?}");
            assert!(matches!(h.state().dialog, Dialog::None));
            let saved = h.state().routing_presets.storage_text();
            h.state_mut().routing_presets = crate::board_presets::Presets::from_storage(saved);
            click_tab(&mut h, "Draw");
            assert_eq!(h.state().views.active.board.tracks.width_nm, expected);
            h.state_mut().views.active.board.tracks.width_nm = 250_000;
            click(&mut h, "Track / line width");
            click(&mut h, &pcbcraft_chrome::units::label(expected, unit));
            assert_eq!(h.state().views.active.board.tracks.width_nm, expected);
            assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Via presets apply as pairs. Invalid custom values, cancellation and stale view input must
/// not change the pair or remember a preset; explicit units work with another display unit.
#[test]
fn pcb_via_dropdown_validates_custom_pairs_and_cancellation() {
    for width in [1400., 760.] {
        let mut app = PcbApp::default();
        app.start_new_project_in(Workspace::Board);
        app.preferences.units = pcbcraft_chrome::units::Unit::Mm;
        let mut h = Harness::builder().with_size(egui::vec2(width, 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let set = |h: &mut Harness<'_, PcbApp>, label: &str, text: &str| {
            click(h, label);
            h.key_press_modifiers(Modifiers::COMMAND, Key::A);
            h.get_by_label(label).type_text(text);
            h.run();
        };
        let pair = |h: &Harness<'_, PcbApp>| {
            let tracks = &h.state().views.active.board.tracks;
            [tracks.via_diameter_nm, tracks.via_drill_nm]
        };
        click(&mut h, "Via pad / drill");
        click(&mut h, "0.8 / 0.4 mm");
        assert_eq!(pair(&h), [800_000, 400_000]);
        let saved = h.state().routing_presets.storage_text();
        click(&mut h, "Via pad / drill");
        click(&mut h, "Custom…");
        set(&mut h, "Pad diameter", "0.7");
        set(&mut h, "Drill diameter", "0.8");
        click(&mut h, "Use");
        assert!(matches!(h.state().dialog, Dialog::RoutingSize(_)));
        assert_eq!(pair(&h), [800_000, 400_000]);
        assert_eq!(h.state().routing_presets.storage_text(), saved);
        h.key_press(Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(pair(&h), [800_000, 400_000]);
        click(&mut h, "Via pad / drill");
        click(&mut h, "Custom…");
        set(&mut h, "Pad diameter", "30 mil");
        set(&mut h, "Drill diameter", "12 mil");
        click(&mut h, "Use");
        assert_eq!(pair(&h), [762_000, 304_800]);
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_ne!(h.state().routing_presets.storage_text(), saved);
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click(&mut h, "Via pad / drill");
        click(&mut h, "Custom…");
        set(&mut h, "Pad diameter", "1.2");
        h.state_mut().session.apply("project.new", &json!({"create_board":true})).unwrap();
        h.run();
        let saved = h.state().routing_presets.storage_text();
        click(&mut h, "Use");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(h.state().routing_presets.storage_text(), saved);
        assert!(h.state().message.as_ref().is_some_and(|message| message.contains("View or project changed")));
    }
}
