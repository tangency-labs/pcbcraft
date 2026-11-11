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
    const RECTANGLE: &str = "Draw rectangle on Board Outline";
    let mut h = harness();
    h.set_size(egui::vec2(1100., 700.));
    resistor_with_board_at_origin(&mut h);
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.get_by_label("Board setup").scroll_to_me();
    h.run();
    let width = h.ctx.content_rect().width() + 20. - h.get_by_label(RECTANGLE).rect().left();
    h.set_size(egui::vec2(width, 700.));
    h.run();
    h.get_by_label("Board setup").scroll_to_me();
    h.run();
    let rect = h.get_by_label(RECTANGLE).rect();
    assert!(h.ctx.content_rect().contains_rect(rect), "the tool stays inside the window: {rect:?}");
    assert!(!crate::tests::fully_visible(&h, RECTANGLE), "the ribbon hides the tool: {rect:?}");
    assert!(!h.get_by_label(RECTANGLE).accesskit_node().is_disabled());
    click(&mut h, RECTANGLE);
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
    for group in ["Place", "Clipboard", "Outline", "Modify", "Align", "Board"] {
        h.get_by_label(&format!("{group} menu"));
    }
    for tool in ["Assign and place component footprints", "Update PCB from schematic", "Board setup"] {
        assert!(!h.get_by_label(tool).accesskit_node().is_disabled());
    }
    for tool in ["Place vias", "Run design rules check", "Measure distance", "Generate fabrication outputs", "View the board from below"] {
        assert!(h.query_by_label(tool).is_none(), "{tool} should have its own task tab");
    }
    click(&mut h, "Board setup");
    assert!(!matches!(h.state().dialog, Dialog::None));
    click(&mut h, "Cancel");
    click(&mut h, "Route");
    h.get_by_label("Place vias");
    h.get_by_label("Route tracks on the active copper layer");
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
        for tab in ["Layout", "Route", "Analyze", "Manufacture", "Libraries", "View"] {
            let rect = h.get_by_role_and_label(egui::Role::Tab, tab).rect();
            assert!(rect.left() >= 0. && rect.right() <= width, "{width}: {tab} {rect:?}");
            click_tab(&mut h, tab);
        }
        click(&mut h, "Route");
        h.get_by_label("Place vias");
    }
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
