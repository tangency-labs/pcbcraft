//! Acceptance for the ported toolbar's command routing and compact presentation.
use super::*;

/// Both promoted buttons and caption menus reach the same sheet/label tools without
/// editing the document until the existing placement form is committed.
#[test]
fn ported_ribbon_reaches_sheet_and_port_workflows_at_both_widths() {
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
            assert!(h.query_by_label("Power symbols are not available yet.").is_none());
            click(&mut h, "Place menu");
            assert!(h.get_by_label("Power").accesskit_node().is_disabled());
            h.key_press(Key::Escape);
            h.run();
        } else {
            assert!(h.get_by_label("Power symbols are not available yet.").accesskit_node().is_disabled());
        }
        // The scrolled button and its caption-menu entry must target the same view.
        h.get_by_label("Place hierarchical sheet (S)").scroll_to_me();
        h.run();
        click(&mut h, "Place hierarchical sheet (S)");
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Hierarchy { kind: crate::hierarchy::Kind::Sheet, .. })));
        h.get_by_label("Select and move items").scroll_to_me();
        h.run();
        click(&mut h, "Select and move items");
        assert!(h.state().views.active.schematic.tool.is_none());
        h.get_by_label("Place menu").scroll_to_me();
        h.run();
        click(&mut h, "Place menu");
        click(&mut h, "Port");
        assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Hierarchy { kind: crate::hierarchy::Kind::Label, .. })));
        h.key_press(Key::V);
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
        click(&mut h, "Undo project edit");
        assert_eq!(h.state().session.project().unwrap().sheets().count(), 1);
        click(&mut h, "Redo project edit");
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
