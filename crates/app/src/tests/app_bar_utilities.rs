//! #336: every workspace ends its app bar with the same application-wide utilities.

use super::*;

/// Switch the focused view through the workspace picker.
fn choose_workspace(h: &mut Harness<'_, PcbApp>, label: &str) {
    click(h, "Choose workspace");
    h.get_by_label(label).scroll_to_me();
    h.run();
    click(h, label);
}

/// The application-wide utilities in the app bar, left to right, with their left edges.
fn utilities(h: &Harness<'_, PcbApp>) -> Vec<(String, i32)> {
    let bar = h.get_by_label("File").rect().bottom();
    let version = format!("v{}", display_version());
    let mut found: Vec<_> = [version.as_str(), "Report a bug", "Help", "Preferences", "Assistant"]
        .into_iter()
        .filter_map(|label| {
            let node = h.query_all_by_label(label).find(|node| node.rect().center().y < bar)?;
            Some((label.to_owned(), node.rect().left().round() as i32))
        })
        .collect();
    found.sort_by_key(|(_, left)| *left);
    found
}

/// Home, Schematic, PCB Layout, both definition editors and the Parts Library show the same
/// utilities in the same places, at full and compact widths.
#[test]
fn every_workspace_shows_the_same_utilities_in_the_same_places() {
    let version = format!("v{}", display_version());
    for (width, expected) in
        [(1000.0, vec!["Assistant", "Preferences", "Help", "Report a bug", version.as_str()]), (360.0, vec!["Assistant", "Preferences"])]
    {
        let mut h = harness();
        h.set_size(egui::vec2(width, 700.0));
        h.run();
        let reference = utilities(&h);
        assert_eq!(reference.iter().map(|(label, _)| label.as_str()).collect::<Vec<_>>(), expected, "Schematic at {width}");
        click(&mut h, "Home");
        assert_eq!(utilities(&h), reference, "Home at {width}");
        h.key_press(Key::Escape);
        h.run();
        for workspace in ["PCB Layout", "Symbol Editor", "Footprint Editor", "Parts Library"] {
            choose_workspace(&mut h, workspace);
            assert_eq!(utilities(&h), reference, "{workspace} at {width}");
        }
    }
}

/// In the library workspaces the utilities reach the same actions and shortcuts as in a project
/// workspace: the Assistant opens over the workspace and Preferences and Help open their dialogs.
#[test]
fn library_workspace_utilities_open_the_assistant_preferences_and_help() {
    let mut h = harness();
    for workspace in ["Symbol Editor", "Parts Library"] {
        choose_workspace(&mut h, workspace);
        click(&mut h, "Assistant");
        assert!(h.state().assistant.toolbar_state().0, "{workspace}");
        h.get_by_label("Close Assistant");
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::A);
        h.run();
        assert!(!h.state().assistant.toolbar_state().0, "{workspace} toggles the Assistant from the keyboard");

        click(&mut h, "Preferences");
        assert!(matches!(h.state().dialog, Dialog::Preferences(_)), "{workspace}");
        h.state_mut().dialog = Dialog::None;
        h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::Preferences(_)), "{workspace} opens Preferences from the keyboard");
        h.state_mut().dialog = Dialog::None;
        h.run();

        choose(&mut h, "Help", "About PCBCraft");
        assert!(matches!(h.state().dialog, Dialog::About), "{workspace}");
        h.state_mut().dialog = Dialog::None;
        h.run();
        choose_workspace(&mut h, "Schematic");
    }
}
