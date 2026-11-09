//! Acceptance for the ported toolbar's command routing and compact presentation.
use super::*;

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
        for (tab, slot) in [("Analyze", 2), ("Libraries", 3), ("Manage", 4), ("View", 1), ("Capture", 0)] {
            click(&mut h, tab);
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

        click(&mut h, "Manage");
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
        for tab in ["Capture", "Analyze", "Libraries", "Manage", "View"] {
            let rect = h.get_by_label(tab).rect();
            assert!(rect.left() >= 0. && rect.right() <= width, "{width}: {tab} {rect:?}");
            click(&mut h, tab);
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

/// The PCB Layout tab follows the layout design's groups. Working tools are enabled buttons;
/// tools that are not built yet are listed, disabled, in their group's menu. Setup and Sync reach
/// the same forms as before, and nothing here edits the project.
#[test]
fn pcb_layout_tab_groups_working_and_planned_tools() {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    resistor_with_board_at_origin(&mut h);
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let before = h.state().session.inspect();
    for group in ["Place", "Outline", "Route", "Modify", "Inspect", "Design"] {
        assert!(h.query_by_label(&format!("{group} menu")).is_some(), "{group} group");
    }
    for tool in ["Assign and place component footprints", "Update PCB from schematic", "Board setup", "Reveal highlighted counterpart"] {
        assert!(!h.get_by_label(tool).accesskit_node().is_disabled(), "{tool} is enabled");
    }
    for (group, planned) in [
        ("Place", &["Zone"][..]),
        ("Route", &["Pair", "Tune"]),
        ("Modify", &["Align"]),
        ("Inspect", &["Rules", "Measure"]),
        ("Design", &["Flip view", "Outputs", "Refill zones"]),
    ] {
        click(&mut h, &format!("{group} menu"));
        for tool in planned {
            assert!(h.get_by_label(tool).accesskit_node().is_disabled(), "{group} › {tool} is planned");
        }
        h.key_press(Key::Escape);
        h.run();
    }
    click(&mut h, "Board setup");
    assert!(!matches!(h.state().dialog, Dialog::None), "Design › Setup opens the board setup form");
    click(&mut h, "Cancel");
    assert_eq!(h.state().session.inspect(), before);
}
