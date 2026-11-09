//! Context actions operate on the owning canvas tool through the existing host handlers.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    Symbols,
    Wire,
    Ground,
    Rotate,
    Repeat(bool),
    FlipWire,
    Cancel,
    SelectAll,
    UnselectAll,
    Fit,
    Grid(i64),
}

/// Placement and wire modes keep their tool while a context popup temporarily owns input.
pub(super) fn supports(tool: Option<&Tool>) -> bool {
    matches!(tool, Some(Tool::PlaceSymbol { .. } | Tool::Wire { .. })) || matches!(tool, Some(Tool::NetLabel { label, .. }) if label.id == 0)
}

/// Offer view actions on empty canvas instead of immediately closing an empty popup.
pub(super) fn idle(ui: &mut Ui, spacing_nm: i64, unit: pcbcraft_chrome::units::Unit) -> Option<Action> {
    let drawing = [("Draw wires", "W", Action::Wire), ("Place symbol…", "A", Action::Symbols), ("Place ground", "", Action::Ground)]
        .into_iter()
        .find_map(|(label, key, action)| MenuItem::new(label).shortcut(key).show(ui).then_some(action));
    if drawing.is_some() {
        return drawing;
    }
    menu::separator(ui);
    let selection = [("Select all", Action::SelectAll), ("Unselect all", Action::UnselectAll)]
        .into_iter()
        .find_map(|(label, action)| MenuItem::new(label).show(ui).then_some(action));
    if selection.is_some() {
        return selection;
    }
    menu::separator(ui);
    if MenuItem::new("Fit schematic").show(ui) {
        return Some(Action::Fit);
    }
    MenuItem::new("Grid")
        .shortcut("›")
        .submenu(ui, |ui| {
            canvas_grid::SCHEMATIC_SPACING_PRESETS_NM.into_iter().find_map(|preset| {
                let label = canvas_grid::spacing_label(preset, unit);
                MenuItem::new(&label).checked(preset == spacing_nm).show(ui).then_some(Action::Grid(preset))
            })
        })
        .and_then(|menu| menu.inner)
}

/// Menus change the placement preference or tool preview; only a later canvas press commits.
pub(super) fn active(ui: &mut Ui, tool: Option<&Tool>, repeat: bool) -> Option<Action> {
    let keys = egui::Modifiers { ctrl: ui.input(|i| i.modifiers.ctrl), ..egui::Modifiers::NONE };
    if ui.input_mut(|i| i.consume_key(keys, Key::Escape)) {
        ui.close();
        return Some(Action::Cancel);
    }
    if matches!(tool, Some(Tool::Wire { .. })) {
        if MenuItem::new("Switch bend direction").shortcut("Space").show(ui) {
            return Some(Action::FlipWire);
        }
    } else {
        if MenuItem::new("Rotate").shortcut("R").show(ui) {
            return Some(Action::Rotate);
        }
        let mut selected = repeat;
        if pcbcraft_chrome::catalog::repeat_placement(ui, &mut selected).changed() {
            ui.close();
            return Some(Action::Repeat(selected));
        }
    }
    MenuItem::new("Cancel").shortcut("Esc").show(ui).then_some(Action::Cancel)
}
