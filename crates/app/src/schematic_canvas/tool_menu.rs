//! Context actions operate on the owning canvas tool through the existing host handlers.
use super::*;
use pcbcraft_chrome::{menu::MenuIcon, workspace::ToolIcon};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    Copy,
    Cut,
    Paste,
    PasteOptions,
    Duplicate,
    MoveToNewSheet,
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
    let select_all_key = ui.ctx().format_shortcut(&crate::SELECT_ALL);
    let selection = [("Select all", select_all_key.as_str(), Action::SelectAll), ("Unselect all", "", Action::UnselectAll)]
        .into_iter()
        .find_map(|(label, key, action)| MenuItem::new(label).shortcut(key).show(ui).then_some(action));
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
                MenuItem::new(&label).choice(preset == spacing_nm).show(ui).then_some(Action::Grid(preset))
            })
        })
        .and_then(|menu| menu.inner)
}

/// Present active tool actions as aligned menu rows, with Cancel in its own group.
/// Actions change the placement preference or tool preview; only a later canvas press commits.
pub(super) fn active(ui: &mut Ui, tool: Option<&Tool>, repeat: bool) -> Option<Action> {
    let keys = egui::Modifiers { ctrl: ui.input(|i| i.modifiers.ctrl), ..egui::Modifiers::NONE };
    if ui.input_mut(|i| i.consume_key(keys, Key::Escape)) {
        ui.close();
        return Some(Action::Cancel);
    }
    if matches!(tool, Some(Tool::Wire { .. })) {
        if MenuItem::new("Switch bend direction").tool_icon(ToolIcon::Wire).shortcut("Space").show(ui) {
            return Some(Action::FlipWire);
        }
    } else {
        if MenuItem::new("Rotate").tool_icon(ToolIcon::Rotate).shortcut("R").show(ui) {
            return Some(Action::Rotate);
        }
        if MenuItem::new("Repeat placement").toggle(repeat).show(ui) {
            return Some(Action::Repeat(!repeat));
        }
    }
    menu::separator(ui);
    MenuItem::new("Cancel").icon(MenuIcon::Close).shortcut("Esc").show(ui).then_some(Action::Cancel)
}

/// Clipboard actions follow the existing selection while Paste always reads the system clipboard.
pub(super) fn clipboard(ui: &mut Ui, selected: bool) -> Option<Action> {
    [
        ("Copy", "Ctrl+C", Action::Copy, selected),
        ("Cut", "Ctrl+X", Action::Cut, selected),
        ("Paste", "Ctrl+V", Action::Paste, true),
        ("Paste Options…", "Ctrl+Shift+V", Action::PasteOptions, true),
        ("Duplicate", "Ctrl+D", Action::Duplicate, selected),
    ]
    .into_iter()
    .find_map(|(label, key, action, enabled)| MenuItem::new(label).shortcut(key).enabled(enabled).show(ui).then_some(action))
}

/// Moving selected symbols or wires to a new child sheet opens a form; nothing changes until Move.
pub(super) fn split(ui: &mut Ui, selected: bool) -> Option<Action> {
    MenuItem::new("Move to New Sheet…").enabled(selected).show(ui).then_some(Action::MoveToNewSheet)
}
