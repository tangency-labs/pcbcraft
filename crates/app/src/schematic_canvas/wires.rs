use egui::{Color32, Painter, PointerButton, Pos2, Rect, Response, Shape, Stroke, pos2};
use pcbcraft_chrome::{Tokens, theme};
use pcbcraft_engine::{Project, schematic::Terminal};

use super::Camera;

/// Paint committed wires from their model endpoints, without inferring connections from crossings.
pub(super) fn paint(painter: &Painter, rect: Rect, project: &Project, camera: &Camera) {
    let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, Tokens::get().schematic_wire);
    for wire in project.wires() {
        let [Some(from), Some(to)] = wire.terminals().map(|pin| project.terminal_position(pin)) else {
            continue;
        };
        let a = camera.screen(from, rect);
        let b = camera.screen(to, rect);
        if rect.intersects(Rect::from_two_pos(a, b).expand(stroke.width)) {
            paint_route(painter, a, b, stroke);
        }
    }
}

/// Highlight the available pins and preview a route. Only a primary click on an exact pin selects it.
pub(super) fn pick(
    painter: &Painter,
    rect: Rect,
    response: &Response,
    project: &Project,
    camera: &Camera,
    start: Option<Terminal>,
) -> Option<Terminal> {
    let tokens = Tokens::get();
    let clicked = response.clicked_by(PointerButton::Primary);
    let pointer = if clicked { response.interact_pointer_pos() } else { response.hover_pos() };
    let pins = project
        .resistors()
        .iter()
        .flat_map(|resistor| resistor.terminals().into_iter().zip(resistor.pins().map(|pin| camera.screen(pin, rect))))
        .filter(|(_, pos)| rect.contains(*pos));
    let target = pointer.and_then(|at| nearest_pin(at, pins.clone()));
    if let Some(from) = start.and_then(|pin| project.terminal_position(pin))
        && let Some(to) = target.map(|(_, pos)| pos).or_else(|| pointer.map(|at| camera.screen(camera.snap(at, rect), rect)))
    {
        paint_route(painter, camera.screen(from, rect), to, Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, tokens.accent));
    }

    for (pin, pos) in pins {
        let active = start == Some(pin) || target.is_some_and(|(target, _)| target == pin);
        let color = if active { tokens.accent } else { tokens.canvas_ink };
        paint_pin(painter, pos, color);
    }
    response.clone().on_hover_cursor(egui::CursorIcon::Crosshair);
    target.filter(|_| clicked).map(|(pin, _)| pin)
}

/// Ignore equally close pins so a click cannot silently choose between overlapping terminals.
fn nearest_pin(pointer: Pos2, pins: impl Iterator<Item = (Terminal, Pos2)>) -> Option<(Terminal, Pos2)> {
    let mut nearest = None;
    let mut nearest_distance = theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2);
    for (pin, pos) in pins {
        let distance = pointer.distance_sq(pos);
        if distance < nearest_distance {
            nearest = Some((pin, pos));
            nearest_distance = distance;
        } else if distance == nearest_distance {
            nearest = None;
        }
    }
    nearest
}

fn paint_pin(painter: &Painter, pos: Pos2, color: Color32) {
    painter.circle_stroke(pos, theme::SCHEMATIC_PIN_RADIUS, Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color));
}

/// Use a symmetric orthogonal display path so the preview matches the committed wire in either direction.
fn paint_route(painter: &Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    if a.x == b.x || a.y == b.y {
        painter.line_segment([a, b], stroke);
    } else {
        let middle_x = (a.x + b.x) * 0.5;
        painter.add(Shape::line(vec![a, pos2(middle_x, a.y), pos2(middle_x, b.y), b], stroke));
    }
}
