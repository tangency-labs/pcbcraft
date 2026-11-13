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

/// Pick enabled named-anchor ink, symbol bodies or the closest wire in that order.
/// Disabled types are ignored before applying precedence.
pub(super) fn at(
    ui: &egui::Ui,
    project: &Project,
    camera: &Camera,
    canvas: Rect,
    point: Pos2,
    filters: crate::selection_filters::Filters,
) -> Selection {
    if let Some(target) = filters.no_connects.then(|| crate::no_connect::marker_at(project, camera, canvas, point)).flatten() {
        Selection::one_no_connect(target)
    } else if let Some(label) = project
        .net_labels()
        .iter()
        .rev()
        .find(|label| filters.allows_net_label(label.style) && crate::net_labels::bounds(ui, camera, canvas, label).contains(point))
    {
        Selection::one_net_label(label.id)
    } else if let Some((symbol, _)) = filters.symbols.then(|| super::symbol_at(project, camera, canvas, point)).flatten() {
        Selection::one_symbol(symbol.id())
    } else if let Some(wire) = filters.wires.then(|| super::wires::at(project, camera, canvas, point)).flatten() {
        Selection::one_wire(wire)
    } else {
        Selection::default()
    }
}

/// Window selection contains whole symbol bodies and complete wire paths; crossing selection
/// touches bodies or actual orthogonal segments, never merely a wire's overall bounding box.
/// Named-net anchors, including power/ground symbols, use their painted glyph/text bounds.
/// Disabled categories are excluded from both containment and crossing selection.
#[allow(clippy::too_many_arguments)]
pub(super) fn boxed(
    ui: &egui::Ui,
    project: &Project,
    camera: &Camera,
    canvas: Rect,
    start: Pos2,
    end: Pos2,
    filters: crate::selection_filters::Filters,
) -> Selection {
    let window = Rect::from_two_pos(start, end);
    let crossing = end.x < start.x;
    let mut hits = Selection::default();
    for symbol in project.symbols().iter().filter(|_| filters.symbols) {
        let body = super::body(symbol.unit_definition(), symbol.turns(), symbol.is_mirrored());
        let center = camera.screen(symbol.position, canvas);
        let body = Rect::from_min_max(center + body.min.to_vec2() * camera.unit_step(), center + body.max.to_vec2() * camera.unit_step());
        if if crossing { window.intersects(body) } else { window.contains_rect(body) } {
            hits.symbols.insert(symbol.id());
        }
    }
    for label in project.net_labels().iter().filter(|label| filters.allows_net_label(label.style)) {
        let bounds = crate::net_labels::bounds(ui, camera, canvas, label);
        if if crossing { window.intersects(bounds) } else { window.contains_rect(bounds) } {
            hits.net_labels.insert(label.object_id());
        }
    }
    for wire in project.wires().iter().filter(|_| filters.wires) {
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
    if filters.no_connects {
        let extent = crate::no_connect::radius(camera.unit_step());
        for (target, at) in project.no_connect_appearances() {
            let center = camera.screen(at, canvas);
            let bounds = Rect::from_center_size(center, egui::vec2(2. * extent, 2. * extent));
            let hit = if crossing {
                window.intersects(bounds)
                    && [-1., 1.].into_iter().any(|sign| {
                        segment_intersects(window, center - egui::vec2(extent, sign * extent), center + egui::vec2(extent, sign * extent))
                    })
            } else {
                window.contains_rect(bounds)
            };
            if hit {
                hits.no_connects.insert(target);
            }
        }
    }
    hits
}

/// Clip a straight X stroke against a selection rectangle; its empty bounding-box corners
/// must not count as crossing hits. Parallel axes are tested without dividing by zero.
fn segment_intersects(rect: Rect, a: Pos2, b: Pos2) -> bool {
    let (mut near, mut far) = (0f32, 1f32);
    for (start, end, min, max) in [(a.x, b.x, rect.min.x, rect.max.x), (a.y, b.y, rect.min.y, rect.max.y)] {
        let delta = end - start;
        if delta.abs() < f32::EPSILON {
            if start < min || start > max {
                return false;
            }
        } else {
            let (t1, t2) = ((min - start) / delta, (max - start) / delta);
            near = near.max(t1.min(t2));
            far = far.min(t1.max(t2));
            if near > far {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests;
