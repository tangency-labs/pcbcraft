//! Editor pane layout and focus presentation. The host owns views, focus and document state.

use egui::{FontId, Rect, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};

use crate::Tokens;

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
    let response = ui.interact(divider, ui.scope_id().with("editor_divider"), Sense::drag());
    response.widget_info(|| WidgetInfo::labeled(Role::GenericContainer, true, "Resize editor panes"));
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

/// Height of a split pane's header, also used to keep workspace cards below it.
pub fn header_height() -> f32 {
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
        area.min.y = (top + header_height() + spacing).min(area.bottom());
    }
    area
}

/// A pane header addresses its own pane, regardless of keyboard focus.
#[derive(Clone, Copy, Debug)]
pub enum PaneAction {
    OpenBeside,
    OpenWindow,
    Detach,
    Dock,
    Close,
    Drag,
}

/// Show the current document path in a split or detached pane's header. Focus changes its
/// underline, leaving the workspace stationary. Inset tabs retain rounded Craft corners.
/// Long paths elide from the left to retain the sheet name;
/// accessibility and hover text always contain the complete path. Return a focus click and
/// an optional view action; the close button dismisses only this pane, and native placement
/// rows and dragging are omitted for browser hosts. A primary drag past egui's threshold returns
/// `Drag`; the host owns the gesture, stable identities and actions applied after rendering.
/// Detached native panes also expose a leading Dock button, independent of header length.
pub fn pane_header(ui: &mut Ui, path: &str, focused: bool, native: bool, detached: bool) -> (bool, Option<PaneAction>) {
    let t = Tokens::get();
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), header_height()), Sense::hover());
    let font = FontId::proportional(t.font_label);
    let measure = |text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), t.text);
    let close_width = t.space_24;
    let dock_width = if native && detached { 2.0 * t.space_24 } else { 0.0 };
    let available = (strip.width() - 2.0 * t.space_4 - 2.0 * t.space_12 - close_width - dock_width).max(0.0);
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
    let width = (galley.size().x + 2.0 * t.space_12 + close_width + dock_width).min(strip.width());
    let tab = Rect::from_min_size(
        strip.min + vec2(t.space_4, t.space_4),
        vec2(width.min((strip.width() - 2.0 * t.space_4).max(0.0)), strip.height() - t.space_4),
    );
    let close_rect = Rect::from_min_max(pos2((tab.right() - close_width).max(tab.left()), tab.top()), tab.max);
    let dock_rect = Rect::from_min_max(tab.min, pos2((tab.left() + dock_width).min(close_rect.left()), tab.bottom()));
    let response = ui
        .interact(
            Rect::from_min_max(dock_rect.right_top(), close_rect.left_bottom()),
            ui.scope_id().with("sheet_tab"),
            if native { Sense::click_and_drag() } else { Sense::click() },
        )
        .on_hover_cursor(if !native {
            egui::CursorIcon::Default
        } else if ui.input(|input| input.pointer.primary_down()) {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, focused, path));
    ui.painter().rect_filled(strip, 0, t.panel);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    let painter = ui.painter_at(tab);
    solidcraft_ui_kit::app_bar::tab_shape(&painter, &solidcraft_theme::Tokens::get(), tab, focused, response.hovered());
    painter.galley(pos2(dock_rect.right() + t.space_12, tab.center().y - galley.size().y / 2.0), galley, t.text);
    painter.line_segment([tab.left_bottom(), tab.right_bottom()], Stroke::new(2.0, if focused { t.accent } else { t.border }));
    let close = solidcraft_ui_kit::icon_button_at(ui, close_rect, "close", &format!("Close view {path}"));
    let mut action = close.on_hover_text("Close view").clicked().then_some(PaneAction::Close);
    if dock_width > 0.0 && dock_rect.width() > t.space_8 {
        let dock = ui.interact(dock_rect, ui.scope_id().with("dock_pane"), Sense::click());
        dock.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Dock in main window"));
        let ink = ui.painter_at(dock_rect);
        let button = dock_rect.shrink(t.space_3);
        ink.rect_filled(button, t.control_radius, if dock.hovered() { t.hover } else { t.panel });
        ink.rect_stroke(button, t.control_radius, Stroke::new(1.0, if dock.has_focus() { t.accent } else { t.border }), egui::StrokeKind::Inside);
        ink.text(dock_rect.center(), egui::Align2::CENTER_CENTER, "Dock", FontId::proportional(t.font_label), t.text);
        if dock.on_hover_text("Dock in main window").clicked() {
            action = Some(PaneAction::Dock);
        }
    }
    if native && response.drag_started_by(egui::PointerButton::Primary) {
        action = Some(PaneAction::Drag);
    }
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
            if crate::controls::button(ui, label).clicked() {
                action = Some(choice);
                ui.close();
            }
        }
    });
    (response.on_hover_text(path).clicked(), action)
}

/// The second half created by docking beside a pane, matching the equal split's orientation.
pub fn dock_region(rect: Rect) -> Rect {
    let gap = Tokens::get().space_6;
    if rect.width() >= 900.0 {
        Rect::from_min_max(pos2(rect.center().x + gap / 2.0, rect.top()), rect.max)
    } else {
        Rect::from_min_max(pos2(rect.left(), rect.center().y + gap / 2.0), rect.max)
    }
}

/// Animate the area a pane will occupy after docking. The preview and label never take input;
/// calling with `active = false` fades the previous highlight while dragging. Ending a drag
/// clears it immediately so a changed split layout cannot retain an obsolete preview.
pub fn dock_preview(ui: &Ui, id: egui::Id, rect: Rect, active: bool, dragging: bool) {
    placement_preview(ui, id, dock_region(rect), "Dock here", active, dragging);
}

/// Mark the entire existing slot that will receive a pane during an in-window move.
pub fn reorder_preview(ui: &Ui, id: egui::Id, rect: Rect, active: bool, dragging: bool) {
    placement_preview(ui, id, rect, "Move pane here", active, dragging);
}

/// Draw passive, animated placement feedback with an accessible label for its actual region.
fn placement_preview(ui: &Ui, id: egui::Id, rect: Rect, caption: &str, active: bool, dragging: bool) {
    let amount = ui.ctx().animate_bool_with_time(id, active, if dragging { 0.12 } else { 0.0 });
    if amount <= 0.0 && !active {
        return;
    }
    let t = Tokens::get();
    let region = rect.shrink(t.space_4);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, id));
    painter.rect_filled(region, t.control_radius, t.accent.gamma_multiply(0.22 * amount));
    painter.rect_stroke(region, t.control_radius, Stroke::new(2.0, t.accent.gamma_multiply(amount)), egui::StrokeKind::Inside);
    if active {
        ui.interact(region, id.with("label"), Sense::hover()).widget_info(|| WidgetInfo::labeled(Role::GenericContainer, true, caption));
        let label = painter.layout_no_wrap(caption.to_owned(), FontId::proportional(t.font_label), t.text);
        let badge = Rect::from_center_size(region.center(), label.size() + vec2(t.space_24, t.space_12));
        painter.rect_filled(badge, t.control_radius, t.panel);
        painter.rect_stroke(badge, t.control_radius, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        painter.galley(badge.center() - label.size() / 2.0, label, t.text);
    }
}

/// An explicit local drop destination. Hosts validate identities against their own instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDrop {
    NewWindow,
    Host(u64),
}

/// Current local pointer and new-window availability for a pane's destination list.
pub struct WindowDropState {
    pub pointer: Option<egui::Pos2>,
    pub can_detach: bool,
    pub detaching: bool,
}

/// Show destinations during every native pane drag, including on compositors that hide
/// window positions. Selection uses the host's captured pointer, including a release after
/// native hover leaves the window; the highlighted row is the operation release will commit.
/// The new-window row also highlights while the captured pointer is outside the client.
/// Anchor beside the initial header press rather than the pane's far edge, which may be off-screen.
pub fn window_drop_targets(
    ui: &Ui,
    pane: Rect,
    anchor: egui::Pos2,
    targets: &[(u64, String)],
    main: u64,
    state: WindowDropState,
) -> Option<WindowDrop> {
    let mut hovered = None;
    let t = Tokens::get();
    let width = (pane.width() - 2.0 * t.space_12).clamp(180.0, 280.0);
    let left = pane.left() + t.space_12;
    let right = (pane.right() - width - t.space_12).max(left);
    egui::Area::new(ui.scope_id().with("window_drop_targets"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos2((anchor.x - width).clamp(left, right), pane.top() + header_height() + t.space_4))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(width);
                let mut choices = Vec::new();
                if state.can_detach {
                    choices.push((WindowDrop::NewWindow, "Open in new window".to_owned()));
                }
                choices.extend(targets.iter().map(|(id, label)| {
                    (
                        WindowDrop::Host(*id),
                        if *id == main { format!("Dock in main window · {label}") } else { format!("Dock in window {id} · {label}") },
                    )
                }));
                for (destination, label) in choices {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, header_height()), Sense::hover());
                    let under_pointer = state.pointer.is_some_and(|pointer| rect.contains(pointer));
                    let selected = under_pointer || (destination == WindowDrop::NewWindow && state.detaching);
                    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &label));
                    let amount = ui.ctx().animate_bool_with_time(response.id, selected, 0.12);
                    ui.painter().rect_filled(rect, t.control_radius, t.accent.gamma_multiply(0.24 * amount));
                    if under_pointer {
                        hovered = Some(destination);
                    }
                    if selected {
                        ui.painter().rect_stroke(rect, t.control_radius, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                    }
                    let mut job = egui::text::LayoutJob::simple(label, FontId::proportional(t.font_label), t.text, width - 2.0 * t.space_8);
                    job.wrap.max_rows = 1;
                    let galley = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(rect.left() + t.space_8, rect.center().y - galley.size().y / 2.0), galley, t.text);
                }
            });
        });
    hovered
}

/// Keep the source pane outlined and shaded until the drag ends, with a brief entrance fade.
pub fn pane_drag_source(ui: &Ui, source: Rect, active: bool) {
    let id = ui.scope_id().with("pane_drag_preview");
    let amount = ui.ctx().animate_bool_with_time(id, active, if active { 0.12 } else { 0.0 });
    if active {
        let t = Tokens::get();
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, id.with("source")));
        painter.rect_filled(source, 0, t.panel.gamma_multiply(0.28 * amount));
        painter.rect_stroke(source.shrink(t.space_4), t.control_radius, Stroke::new(1.0, t.accent.gamma_multiply(amount)), egui::StrokeKind::Inside);
    }
}
