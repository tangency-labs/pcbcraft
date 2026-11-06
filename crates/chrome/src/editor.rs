//! Editor pane layout and focus presentation. The host owns views, focus and document state.

use egui::{FontId, Rect, Sense, Stroke, Ui, WidgetInfo, WidgetType, pos2, vec2};

use crate::Tokens;

/// User intent from the arrangement tab strip, applied by the project host after rendering.
#[derive(Clone, Debug)]
pub enum ArrangementAction {
    Select(u64),
    New,
    Duplicate,
    Rename(String),
    Remove,
}

/// Draw named arrangements within the title bar, with document-view controls remaining in panes.
/// Overflow scrolls horizontally; naming and removal stay in the explicit arrangement menu.
pub fn arrangement_tabs(ui: &mut Ui, tabs: &[(u64, String)], active: u64, rename: &mut String) -> Option<ArrangementAction> {
    if tabs.is_empty() {
        return None;
    }
    let t = Tokens::get();
    let mut action = None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - t.space_24 - t.space_8).max(0.0);
        egui::ScrollArea::horizontal()
            .id_salt("arrangement_tabs")
            .max_width(width)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (id, name) in tabs {
                        let response = ui.selectable_label(*id == active, name);
                        response
                            .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, *id == active, format!("Arrangement {name}")));
                        if response.clicked() {
                            action = Some(ArrangementAction::Select(*id));
                        }
                    }
                });
            });
        let menu = ui.menu_button("+", |ui| {
            if ui.button("New arrangement").clicked() {
                action = Some(ArrangementAction::New);
                ui.close();
            }
            if ui.button("Duplicate arrangement").clicked() {
                action = Some(ArrangementAction::Duplicate);
                ui.close();
            }
            ui.menu_button("Rename arrangement", |ui| {
                ui.text_edit_singleline(rename).labelled_by(ui.label("Name").id);
                if ui.button("Apply name").clicked() {
                    action = Some(ArrangementAction::Rename(rename.clone()));
                    ui.close();
                }
            });
            if ui.add_enabled(tabs.len() > 1, egui::Button::new("Remove arrangement")).clicked() {
                action = Some(ArrangementAction::Remove);
                ui.close();
            }
        });
        menu.response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Arrangement options"));
        menu.response.on_hover_text("Arrangements");
    });
    action
}

/// Lay out two panes with a draggable divider. Narrow windows stack them vertically so each
/// retains usable width. The ratio is presentation state; dragging clamps it to 25–75 percent.
pub fn split(ui: &mut Ui, ratio: &mut f32) -> [Rect; 2] {
    let t = Tokens::get();
    let rect = ui.available_rect_before_wrap();
    let horizontal = rect.width() >= 900.0;
    let extent = if horizontal { rect.width() } else { rect.height() };
    let gap = t.space_6.min(extent);
    let position = |ratio: f32| (extent - gap) * ratio;
    let at = position(*ratio);
    let divider = if horizontal {
        Rect::from_min_max(pos2(rect.left() + at, rect.top()), pos2(rect.left() + at + gap, rect.bottom()))
    } else {
        Rect::from_min_max(pos2(rect.left(), rect.top() + at), pos2(rect.right(), rect.top() + at + gap))
    };
    let response = ui.interact(divider, ui.id().with("editor_divider"), Sense::drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Resize editor panes"));
    if response.dragged() && extent > gap {
        let delta = if horizontal { response.drag_delta().x } else { response.drag_delta().y };
        *ratio = (*ratio + delta / (extent - gap)).clamp(0.25, 0.75);
    }
    response.on_hover_cursor(if horizontal { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
    let at = position(*ratio);
    if horizontal {
        let x = rect.left() + at;
        ui.painter().rect_filled(Rect::from_min_max(pos2(x, rect.top()), pos2(x + gap, rect.bottom())), 0, t.border);
        [Rect::from_min_max(rect.min, pos2(x, rect.bottom())), Rect::from_min_max(pos2(x + gap, rect.top()), rect.max)]
    } else {
        let y = rect.top() + at;
        ui.painter().rect_filled(Rect::from_min_max(pos2(rect.left(), y), pos2(rect.right(), y + gap)), 0, t.border);
        [Rect::from_min_max(rect.min, pos2(rect.right(), y)), Rect::from_min_max(pos2(rect.left(), y + gap), rect.max)]
    }
}

/// Height of a document tab strip, also used to keep workspace cards below the first strip.
pub fn tab_height() -> f32 {
    let t = Tokens::get();
    t.space_24 + t.space_8
}

/// Keep shared edge cards below the top pane headers and above any lower row of panes.
/// Their horizontal anchors span the host and do not move when a different pane takes focus.
pub fn panel_area(mut area: Rect, panes: &[Rect], headers: bool, spacing: f32) -> Rect {
    if headers {
        let top = area.top();
        for pane in panes {
            if pane.top() > top {
                area.max.y = area.bottom().min(pane.top() - spacing);
            }
        }
        area.min.y = (top + tab_height() + spacing).min(area.bottom());
    }
    area
}

/// A document tab addresses its own view, regardless of keyboard focus.
#[derive(Clone, Copy, Debug)]
pub enum PaneAction {
    OpenBeside,
    OpenWindow,
    Detach,
    Dock,
    Close,
}

/// Draw one pane's document tabs with stable view identities and horizontal overflow.
/// Selecting or closing a tab returns intent for that view, independently of the host's focus.
pub fn document_tabs(
    ui: &mut Ui,
    tabs: &[(u64, String)],
    selected: u64,
    focused: bool,
    native: bool,
    detached: bool,
) -> (Option<u64>, Option<(u64, PaneAction)>) {
    let t = Tokens::get();
    let mut picked = None;
    let mut action = None;
    egui::ScrollArea::horizontal()
        .id_salt("documents")
        .max_height(tab_height())
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                for (id, path) in tabs {
                    let width = ui.painter().layout_no_wrap(path.clone(), FontId::proportional(t.font_label), t.text).size().x
                        + 2.0 * t.space_12
                        + t.space_24;
                    let width = width.min(t.space_24 * 12.0);
                    ui.push_id(id, |ui| {
                        ui.allocate_ui_with_layout(vec2(width, tab_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                            let (click, intent) = sheet_tab(ui, path, *id == selected, focused, native, detached);
                            if click {
                                picked = Some(*id);
                            }
                            if let Some(intent) = intent {
                                action = Some((*id, intent));
                            }
                        });
                    });
                }
            });
        });
    (picked, action)
}

/// Show the sheet path on a document tab. Focus changes its underline, leaving the tab and
/// surrounding workspace stationary. Long paths elide from the left to retain the sheet name;
/// accessibility and hover text always contain the complete path. Return a focus click and
/// an optional view action; the close button dismisses only this pane, and native placement
/// rows are omitted for browser hosts. The host supplies stable IDs and applies actions after rendering.
pub fn sheet_tab(ui: &mut Ui, path: &str, active: bool, focused: bool, native: bool, detached: bool) -> (bool, Option<PaneAction>) {
    let t = Tokens::get();
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), tab_height()), Sense::hover());
    let font = FontId::proportional(t.font_label);
    let measure = |text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), t.text);
    let close_width = t.space_24;
    let available = (strip.width() - 2.0 * t.space_12 - close_width).max(0.0);
    let mut label = path.to_owned();
    if measure(&label).size().x > available {
        for (index, _) in path.char_indices().skip(1) {
            label = format!("…{}", &path[index..]);
            if measure(&label).size().x <= available {
                break;
            }
        }
    }
    let galley = measure(&label);
    let width = (galley.size().x + 2.0 * t.space_12 + close_width).min(strip.width());
    let tab = Rect::from_min_size(strip.min, vec2(width, strip.height()));
    let close_rect = Rect::from_min_max(pos2((tab.right() - close_width).max(tab.left()), tab.top()), tab.max);
    let response = ui.interact(Rect::from_min_max(tab.min, close_rect.left_bottom()), ui.id().with("sheet_tab"), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, active, path));
    ui.painter().rect_filled(strip, 0, t.panel);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    let painter = ui.painter_at(tab);
    painter.rect_filled(tab, 0, if active { t.canvas } else { t.panel });
    painter.galley(pos2(tab.left() + t.space_12, tab.center().y - galley.size().y / 2.0), galley, t.text);
    painter.line_segment([tab.left_bottom(), tab.right_bottom()], Stroke::new(2.0, if active && focused { t.accent } else { t.border }));
    let close = ui.interact(close_rect, ui.id().with("close_pane"), Sense::click());
    close.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, format!("Close view {path}")));
    if close.hovered() || close.has_focus() {
        painter.rect_filled(close_rect.shrink(t.space_4), t.control_radius, t.hover);
    }
    let center = close_rect.center();
    for direction in [-1.0, 1.0] {
        let arm = vec2(t.space_3, direction * t.space_3);
        painter.line_segment([center - arm, center + arm], Stroke::new(1.2, t.text_dim));
    }
    let mut action = close.on_hover_text("Close view").clicked().then_some(PaneAction::Close);
    response.context_menu(|ui| {
        let mut choices = vec![("Open beside", PaneAction::OpenBeside)];
        if native {
            choices.push(("Open in new window", PaneAction::OpenWindow));
            choices.push(("Detach view", PaneAction::Detach));
            if detached {
                choices.push(("Dock in main window", PaneAction::Dock));
            }
        }
        choices.push(("Close view", PaneAction::Close));
        for (label, choice) in choices {
            if ui.button(label).clicked() {
                action = Some(choice);
                ui.close();
            }
        }
    });
    (response.on_hover_text(path).clicked(), action)
}
