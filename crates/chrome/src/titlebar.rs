//! Window interactions for desktop hosts that draw their own title bar.

use egui::{CursorIcon, Id, LayerId, Order, PointerButton, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};

use crate::Tokens;

/// Native window interactions share a layer raised above project modals in each viewport.
pub(crate) fn interaction_layer() -> LayerId {
    LayerId::new(Order::Foreground, Id::new("native-window-interactions"))
}

/// Keep native-grab recovery local to the viewport whose pointer the OS took over.
fn native_grab_id(ctx: &egui::Context) -> Id {
    Id::new(("native-window-grab", ctx.viewport_id()))
}

/// Hand ownership to the OS and remember that its grab may consume the pointer release.
fn begin_native_grab(ctx: &egui::Context, command: ViewportCommand) {
    ctx.send_viewport_cmd(command);
    ctx.stop_dragging();
    let id = native_grab_id(ctx);
    ctx.data_mut(|data| data.insert_temp(id, true));
}

/// Recover only our own native grabs. Wayland may send PointerGone instead of a release;
/// clearing that departed pointer prevents stale buttons from suppressing the next edge hover.
/// Ordinary editor drags and normally delivered releases retain their input/click history.
fn finish_native_grab(ctx: &egui::Context) {
    let id = native_grab_id(ctx);
    if !ctx.data(|data| data.get_temp::<bool>(id).unwrap_or(false)) {
        return;
    }
    let (released, departed) = ctx.input(|input| (input.pointer.primary_released(), !input.pointer.has_pointer() || !input.focused));
    if departed {
        ctx.input_mut(|input| input.pointer = egui::PointerState::default());
        ctx.stop_dragging();
    }
    if released || departed {
        ctx.data_mut(|data| data.remove::<bool>(id));
    }
}

/// Hand the initial primary press to the OS immediately, before egui's drag threshold.
/// Waiting for motion can lose the native press serial. Double clicks toggle maximization.
pub(crate) fn drag_area(ui: &mut Ui, rect: Rect, region: &'static str) {
    if !rect.is_positive() {
        return;
    }
    let native = ui.new_child(egui::UiBuilder::new().layer_id(interaction_layer()).max_rect(rect));
    let response = native.interact(rect, ui.id().with(("window-drag", region)), Sense::click_and_drag());
    if response.double_clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!is_maximized(ui)));
    } else if response.is_pointer_button_down_on() && ui.input(|input| input.pointer.button_pressed(PointerButton::Primary)) {
        begin_native_grab(ui.ctx(), ViewportCommand::StartDrag);
    }
}

/// Read maximized state from the OS so the control also tracks system shortcuts.
fn is_maximized(ui: &Ui) -> bool {
    ui.input(|input| input.viewport().maximized.unwrap_or(false))
}

/// Add the window controls from right to left, using narrower targets in the compact bar.
/// Maximize becomes Restore when the OS reports maximized.
pub(crate) fn window_controls(ui: &mut Ui, t: Tokens, compact: bool) {
    let maximized = is_maximized(ui);
    let maximize = if maximized { "Restore window" } else { "Maximize window" };
    for (label, command) in [
        ("Close window", ViewportCommand::Close),
        (maximize, ViewportCommand::Maximized(!maximized)),
        ("Minimize window", ViewportCommand::Minimized(true)),
    ] {
        let width = if compact { t.space_24 } else { t.control_height };
        let (rect, response) = ui.allocate_exact_size(vec2(width, t.control_height), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, t.radius_sm, t.hover);
        }
        let center = rect.center();
        let at = |x, y| center + vec2(x, y);
        let stroke = Stroke::new(1.0, t.text);
        let painter = ui.painter();
        match command {
            ViewportCommand::Close => {
                painter.line_segment([at(-4.0, -4.0), at(4.0, 4.0)], stroke);
                painter.line_segment([at(-4.0, 4.0), at(4.0, -4.0)], stroke);
            }
            ViewportCommand::Maximized(false) => {
                painter.line([at(-2.0, -2.0), at(-2.0, -5.0), at(5.0, -5.0), at(5.0, 2.0), at(2.0, 2.0)].to_vec(), stroke);
                painter.rect_stroke(Rect::from_min_max(at(-5.0, -2.0), at(2.0, 5.0)), 0, stroke, egui::StrokeKind::Middle);
            }
            ViewportCommand::Maximized(true) => {
                painter.rect_stroke(Rect::from_min_max(at(-4.0, -4.0), at(4.0, 4.0)), 0, stroke, egui::StrokeKind::Middle);
            }
            _ => {
                painter.line_segment([at(-4.0, 0.0), at(4.0, 0.0)], stroke);
            }
        }
        if response.on_hover_text(label).clicked() {
            ui.ctx().send_viewport_cmd(command);
        }
    }
}

/// Restore OS resizing along the four edges and corners of an undecorated window.
/// Maximized and fullscreen windows have no active resize zones.
/// Release egui's drag ownership when handing the gesture to the OS: Wayland may consume
/// the release event, and retaining the drag would block subsequent edge hover and presses.
/// Call on an enabled UI after drawing dialogs so native controls, dragging and resizing remain
/// above modal backdrops without enabling any application actions behind those dialogs.
pub fn resize_window_edges(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    finish_native_grab(&ctx);
    ctx.move_to_top(interaction_layer());
    if is_maximized(ui) || ui.input(|input| input.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let window = ctx.content_rect();
    let edge = Tokens::get().space_6 / ctx.zoom_factor();
    let corner = 2.0 * edge;
    let (l, r, t, b) = (window.left(), window.right(), window.top(), window.bottom());
    use ResizeDirection::*;
    let zones = [
        (pos2(l, t), pos2(r, t + edge), North, CursorIcon::ResizeVertical),
        (pos2(l, b - edge), pos2(r, b), South, CursorIcon::ResizeVertical),
        (pos2(l, t), pos2(l + edge, b), West, CursorIcon::ResizeHorizontal),
        (pos2(r - edge, t), pos2(r, b), East, CursorIcon::ResizeHorizontal),
        (pos2(l, t), pos2(l + corner, t + corner), NorthWest, CursorIcon::ResizeNwSe),
        (pos2(r - corner, t), pos2(r, t + corner), NorthEast, CursorIcon::ResizeNeSw),
        (pos2(l, b - corner), pos2(l + corner, b), SouthWest, CursorIcon::ResizeNeSw),
        (pos2(r - corner, b - corner), pos2(r, b), SouthEast, CursorIcon::ResizeNwSe),
    ];
    let resize = ui.new_child(egui::UiBuilder::new().layer_id(interaction_layer()).max_rect(window));
    for (index, (min, max, direction, cursor)) in zones.into_iter().enumerate() {
        let response = resize.interact(Rect::from_min_max(min, max), Id::new(("window-resize", index)), Sense::drag());
        if response.on_hover_cursor(cursor).drag_started_by(PointerButton::Primary) {
            begin_native_grab(&ctx, ViewportCommand::BeginResize(direction));
        }
    }
}
