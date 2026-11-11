//! Floating Assistant chrome. The caller owns conversation and editor state.

use egui::{Align, Align2, Area, Context, FontId, Frame, Layout, Margin, Order, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, vec2};

use crate::Tokens;

const ENTRY_WIDTH: f32 = 260.0;
const ENTRY_HEIGHT: f32 = 40.0;
const MAX_DRAWER_WIDTH: f32 = 572.0;

/// Leave room for the Assistant launcher below other cards in the right rail.
pub fn rail_clearance() -> f32 {
    ENTRY_HEIGHT + Tokens::get().space_16
}

/// The expanded Assistant header's window and conversation commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Close,
    Minimize,
    NewChat,
    PopOut,
    Dock,
}

/// The workspace edge and optional user-selected width of the floating panel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Dock {
    pub left: bool,
    pub width: Option<f32>,
}

/// Paint the title bar launcher next to Preferences, including its active state and shortcut.
pub fn toolbar(ui: &mut Ui, active: bool, ready: bool, shortcut: &str) -> bool {
    let t = Tokens::get();
    let (rect, response) = icon_target(ui, "Assistant", &format!("Assistant ({shortcut})"), active);
    paint_glyph(ui.painter(), rect.center(), t.text, icon_fill(&response, t, active));
    if ready {
        ui.painter().circle_filled(rect.right_top() + vec2(-t.space_4, t.space_4), t.space_3, t.accent);
    }
    response.clicked()
}

/// Draw the bottom-right strip. Clicking it restores the Assistant.
pub fn minimized(ctx: &Context, workspace: Rect) -> bool {
    let t = Tokens::get();
    let rect = entry_bounds(workspace);
    let width = rect.width();
    Area::new(egui::Id::unique("assistant_minimized"))
        .fixed_pos(rect.min)
        .constrain_to(workspace)
        .default_size(vec2(width, ENTRY_HEIGHT))
        .order(Order::Foreground)
        .show(ctx, |ui| launcher(ui, width, t))
        .inner
}

/// The minimized Assistant's footprint, for navigation that avoids floating controls.
pub fn entry_bounds(workspace: Rect) -> Rect {
    let bounds = workspace.shrink(Tokens::get().space_12);
    let size = vec2(bounds.width().min(ENTRY_WIDTH), ENTRY_HEIGHT);
    Rect::from_min_size(bounds.right_bottom() - size, size)
}

/// The expanded Assistant's footprint on either workspace edge.
pub fn drawer_bounds(workspace: Rect, dock: Dock) -> Rect {
    dock_bounds(workspace.shrink(Tokens::get().space_12), dock)
}

/// Show the full-height workspace panel, with a drag handle for docking and an inner resize edge.
pub fn drawer(ctx: &Context, workspace: Rect, dock: &mut Dock, status: &str, ready: bool, add: impl FnOnce(&mut Ui)) -> (Option<Action>, Rect) {
    let t = Tokens::get();
    let bounds = workspace.shrink(t.space_12);
    let rect = drawer_bounds(workspace, *dock);
    let left = dock.left;
    let shown = Area::new(egui::Id::unique("assistant_drawer"))
        .fixed_pos(rect.min)
        .movable(false)
        .constrain_to(workspace)
        .default_size(rect.size())
        .order(Order::Foreground)
        .show(ctx, |ui| {
            solidcraft_ui_kit::panel_host_frame(ctx, true)
                .inner_margin(Margin::same(t.space_12 as i8))
                .show(ui, |ui| {
                    let inset = 2.0 * (t.space_12 + 1.0);
                    ui.set_width(rect.width() - inset);
                    ui.set_height(rect.height() - inset);
                    let action = header(ui, status, ready, Some((dock, bounds)));
                    ui.separator();
                    add(ui);
                    action
                })
                .inner
        });
    let edge = if left { rect.right() } else { rect.left() };
    let grip = Rect::from_center_size(egui::pos2(edge, rect.center().y), vec2(t.space_8, rect.height()));
    // Keep the resize grip on the panel layer so it wins over the schematic canvas.
    Area::new(egui::Id::unique("assistant_resize_grip")).fixed_pos(grip.min).movable(false).order(Order::Foreground).show(ctx, |ui| {
        let (_, resize) = ui.allocate_exact_size(grip.size(), Sense::drag());
        if resize.hovered() || resize.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resize.dragged()
            && let Some(pointer) = resize.interact_pointer_pos()
        {
            let width = if left { pointer.x - bounds.left() } else { bounds.right() - pointer.x };
            let width = width.clamp(280.0, MAX_DRAWER_WIDTH);
            if dock.width != Some(width) {
                dock.width = Some(width);
                ctx.request_repaint();
            }
        }
    });
    (shown.inner, shown.response.rect)
}

/// Keep the floating bounds inside the workspace on either side.
fn dock_bounds(bounds: Rect, dock: Dock) -> Rect {
    let width = dock.width.unwrap_or_else(|| (bounds.width() * 0.36).clamp(360.0, MAX_DRAWER_WIDTH)).min(bounds.width());
    let x = if dock.left { bounds.left() } else { bounds.right() - width };
    Rect::from_min_size(egui::pos2(x, bounds.top()), vec2(width, bounds.height()))
}

/// Draw the detached window's content with the same conversation controls.
pub fn window(ui: &mut Ui, status: &str, ready: bool, add: impl FnOnce(&mut Ui)) -> Option<Action> {
    let t = Tokens::get();
    Frame::new()
        .fill(t.panel)
        .inner_margin(Margin::same(t.space_12 as i8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size().max(vec2(0.0, 0.0)));
            let action = header(ui, status, ready, None);
            ui.separator();
            add(ui);
            action
        })
        .inner
}

/// Lay out SolidCraft's header actions. The title doubles as a docking grip in the workspace.
fn header(ui: &mut Ui, status: &str, ready: bool, dock: Option<(&mut Dock, Rect)>) -> Option<Action> {
    let t = Tokens::get();
    let size = vec2(ui.available_width(), t.control_height);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
        let attached = dock.is_some();
        let (grip, _) = ui.allocate_exact_size(vec2(t.space_8, t.space_16), Sense::hover());
        if attached {
            for x in [-1.5, 1.5] {
                for y in [-4.0, 0.0, 4.0] {
                    ui.painter().circle_filled(grip.center() + vec2(x, y), 0.8, t.text_dim);
                }
            }
        }
        let title = ui.label(egui::RichText::new("Assistant").font(crate::fonts::semibold(t.font_button)));
        if let Some((dock, bounds)) = dock {
            let drag = ui.interact(grip.union(title.rect), ui.scope_id().with("assistant_dock"), Sense::drag());
            dock_drag(ui, &drag, dock, bounds);
        }
        if size.x >= 275.0 {
            badge(ui, status, ready);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut actions = [None; 4];
            actions[0] = action_button(ui, Action::Close, "Close Assistant", "Hide the Assistant");
            if attached {
                actions[1] = action_button(ui, Action::Minimize, "Minimize Assistant", "Fold the Assistant into a strip");
            }
            actions[2] = action_button(ui, Action::NewChat, "New chat", "Start a new chat");
            actions[3] = if attached {
                action_button(ui, Action::PopOut, "Pop out Assistant", "Open in its own window")
            } else {
                action_button(ui, Action::Dock, "Dock Assistant", "Return to the workspace")
            };
            actions.into_iter().flatten().next()
        })
        .inner
    })
    .inner
}

/// Preview the destination while dragging the title, and switch sides on release.
fn dock_drag(ui: &Ui, drag: &egui::Response, dock: &mut Dock, bounds: Rect) {
    if drag.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let Some(pointer) = drag.interact_pointer_pos().or_else(|| ui.ctx().pointer_latest_pos()) else { return };
    let left = pointer.x < bounds.center().x;
    if drag.dragged() {
        let t = Tokens::get();
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let target = dock_bounds(bounds, Dock { left, ..*dock });
        let painter = ui.ctx().layer_painter(egui::LayerId::new(Order::Tooltip, egui::Id::unique("assistant_dock_preview")));
        painter.rect(target, t.radius_lg, t.accent.gamma_multiply(0.18), Stroke::new(2.0, t.accent), StrokeKind::Inside);
        painter.text(
            target.center(),
            Align2::CENTER_CENTER,
            if left { "Dock on the left" } else { "Dock on the right" },
            FontId::proportional(t.font_button),
            t.text,
        );
    }
    if drag.drag_stopped() {
        dock.left = left;
        ui.ctx().request_repaint();
    }
}

/// Paint a header glyph and expose its action as a keyboard-accessible button.
fn action_button(ui: &mut Ui, action: Action, label: &str, tooltip: &str) -> Option<Action> {
    let t = Tokens::get();
    let (rect, response) = icon_target(ui, label, tooltip, false);
    let c = rect.center();
    let stroke = Stroke::new(1.3, t.text);
    let p = ui.painter();
    let line = |from, to| p.line_segment([c + from, c + to], stroke);
    match action {
        Action::Close => {
            line(vec2(-4.0, -4.0), vec2(4.0, 4.0));
            line(vec2(-4.0, 4.0), vec2(4.0, -4.0));
        }
        Action::Minimize => chevron(p, c, 1.0, t.text_dim),
        Action::NewChat => {
            p.rect_stroke(Rect::from_center_size(c, vec2(13.0, 10.0)), 2, stroke, StrokeKind::Inside);
            line(vec2(-3.0, 0.0), vec2(3.0, 0.0));
            line(vec2(0.0, -3.0), vec2(0.0, 3.0));
        }
        Action::PopOut | Action::Dock => {
            p.rect_stroke(Rect::from_center_size(c, vec2(11.0, 10.0)), 1, stroke, StrokeKind::Inside);
            let sign = if action == Action::PopOut { 1.0 } else { -1.0 };
            let tip = vec2(6.0, -6.0) * sign;
            p.rect_filled(Rect::from_center_size(c + tip, vec2(7.0, 7.0)), 0, icon_fill(&response, t, false));
            line(vec2(0.0, 0.0), tip);
            line(tip, tip + vec2(-4.0, 0.0) * sign);
            line(tip, tip + vec2(0.0, 4.0) * sign);
        }
    }
    response.clicked().then_some(action)
}

/// Allocate a compact, labelled button shared by the toolbar and window header.
fn icon_target(ui: &mut Ui, label: &str, tooltip: &str, active: bool) -> (Rect, egui::Response) {
    let t = Tokens::get();
    let (_, rect) = ui.allocate_space(vec2(t.control_height, t.control_height));
    let response = crate::controls::glyph_target(ui, ui.scope_id().with(("assistant_action", label)), rect, label, ui.is_enabled(), active)
        .on_hover_text(tooltip);
    (rect, response)
}

/// Match the glyph's background to the button surface.
fn icon_fill(response: &egui::Response, t: Tokens, active: bool) -> egui::Color32 {
    if response.hovered() || response.has_focus() {
        t.hover
    } else if active {
        t.panel_header
    } else {
        t.panel
    }
}

/// Render the shared Assistant status pill using its ready or neutral semantic tone.
fn badge(ui: &mut Ui, label: &str, ready: bool) {
    solidcraft_ui_kit::assistant_badge(
        ui,
        label,
        if ready { solidcraft_ui_kit::AssistantTone::Success } else { solidcraft_ui_kit::AssistantTone::Neutral },
    );
}

/// Render the launcher icon, title and up chevron inside one accessible hit target.
fn launcher(ui: &mut Ui, width: f32, t: Tokens) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(width, ENTRY_HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Expand Assistant"));
    let painter = ui.painter();
    let fill = if response.hovered() || response.has_focus() { t.hover } else { t.panel };
    painter.rect(rect, t.radius_lg, fill, Stroke::new(1.0, t.border), StrokeKind::Inside);
    let icon = rect.left_center() + vec2(22.0, 0.0);
    paint_glyph(painter, icon, t.text, fill);
    painter.text(icon + vec2(22.0, 0.0), Align2::LEFT_CENTER, "Assistant", FontId::proportional(t.font_body), t.text);
    let arrow = rect.right_center() - vec2(20.0, 0.0);
    chevron(painter, arrow, -1.0, t.text_dim);
    response.clicked()
}

/// Paint the chevron independently of the font's glyph coverage.
fn chevron(painter: &egui::Painter, center: Pos2, direction: f32, ink: egui::Color32) {
    let dy = direction * 2.0;
    painter.add(Shape::line(vec![center + vec2(-4.0, -dy), center + vec2(0.0, dy), center + vec2(4.0, -dy)], Stroke::new(1.3, ink)));
}

/// Draw the agent switcher. The caller supplies the available agents and handles selection.
pub fn agent_switcher(ui: &mut Ui, label: &str, enabled: bool, add: impl FnOnce(&mut Ui)) {
    ui.add_enabled_ui(enabled, |ui| {
        crate::controls::Choice::from_id_salt("assistant_agent")
            .selected_text(label)
            .width(ui.available_width().min(260.0))
            .show_ui(ui, add)
            .response
            .on_hover_text("Choose an agent");
    });
}

/// Draw the family assistant bubble and sparkles using the current theme.
fn paint_glyph(painter: &egui::Painter, center: Pos2, ink: egui::Color32, fill: egui::Color32) {
    let at = |x, y| center + vec2(x, y);
    painter.rect_filled(Rect::from_min_max(at(-7.0, -6.0), at(7.0, 5.0)), 2, ink);
    painter.add(Shape::convex_polygon(vec![at(-5.0, 3.0), at(-1.0, 5.0), at(-5.0, 8.0)], ink, Stroke::NONE));
    for (spark, size) in [(at(-1.5, -0.5), 3.0), (at(3.0, -3.5), 1.4)] {
        painter.add(Shape::convex_polygon(
            vec![spark - vec2(size, 0.0), spark - vec2(0.0, size), spark + vec2(size, 0.0), spark + vec2(0.0, size)],
            fill,
            Stroke::NONE,
        ));
    }
}

/// Pin input and actions in the shared composer surface below the scrolling conversation.
#[cfg(not(target_arch = "wasm32"))]
pub fn composer(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let focused = ui.memory(|memory| memory.has_focus(egui::Id::unique("assistant_draft_input")));
    egui::Panel::bottom("assistant_composer").resizable(false).show_separator_line(false).frame(Frame::NONE).show(ui, |ui| {
        solidcraft_ui_kit::assistant_composer(ui, focused, add);
    });
}

/// Place the user's message in a right-aligned bubble within the conversation.
#[cfg(not(target_arch = "wasm32"))]
pub fn user_bubble(ui: &mut Ui, message: &str) {
    solidcraft_ui_kit::assistant_bubble(ui, |ui| {
        ui.add(egui::Label::new(message).wrap());
    });
}
