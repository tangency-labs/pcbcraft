//! Disposable box gestures and hit testing over the same bodies and paths used by the canvas.

use super::Camera;
use egui::{PointerButton, Pos2, Rect};
use pcbcraft_engine::{Project, selection::Selection};

#[derive(Clone, Debug)]
pub(crate) struct BoxDrag {
    pub start: Pos2,
    pub base: Selection,
    pub add: bool,
    pub toggle: bool,
    pub button: PointerButton,
}

/// Pick one symbol body or closest wire, with the same precedence as a plain canvas click.
pub(super) fn at(project: &Project, camera: &Camera, canvas: Rect, point: Pos2) -> Selection {
    if let Some((symbol, _)) = super::symbol_at(project, camera, canvas, point) {
        Selection::one_symbol(symbol.id())
    } else if let Some(wire) = super::wires::at(project, camera, canvas, point) {
        Selection::one_wire(wire)
    } else {
        Selection::default()
    }
}

/// Window selection contains whole symbol bodies and complete wire paths; crossing selection
/// touches bodies or actual orthogonal segments, never merely a wire's overall bounding box.
pub(super) fn boxed(project: &Project, camera: &Camera, canvas: Rect, start: Pos2, end: Pos2) -> Selection {
    let window = Rect::from_two_pos(start, end);
    let crossing = end.x < start.x;
    let mut hits = Selection::default();
    for symbol in project.symbols() {
        let body = super::body(&symbol.definition, symbol.turns());
        let center = camera.screen(symbol.position, canvas);
        let body = Rect::from_min_max(center + body.min.to_vec2() * camera.unit_step(), center + body.max.to_vec2() * camera.unit_step());
        if if crossing { window.intersects(body) } else { window.contains_rect(body) } {
            hits.symbols.insert(symbol.id());
        }
    }
    for wire in project.wires() {
        let path: Vec<_> = project.wire_path(wire).into_iter().map(|point| camera.screen(point, canvas)).collect();
        let hit = if crossing {
            path.windows(2).any(|pair| window.intersects(Rect::from_two_pos(pair[0], pair[1])))
        } else {
            !path.is_empty() && path.iter().all(|point| window.contains(*point))
        };
        if hit {
            hits.wires.insert(wire.id());
        }
    }
    hits
}

#[cfg(test)]
mod tests;
