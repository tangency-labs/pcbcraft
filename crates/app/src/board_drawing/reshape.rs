//! Outline geometry shared by board graphics and copper zones.
use super::{Drawing, Point};
use crate::canvas_camera::Camera;
use egui::{Pos2, Rect};

const VERTEX_RADIUS: f32 = 8.;
const ALIGN_RADIUS: f32 = 6.;

#[derive(Clone, Copy, Debug)]
pub(super) enum Handle {
    Vertex(usize),
    Edge(usize),
}

#[derive(Clone, Copy)]
pub(super) struct Snap {
    pub(super) point: Point,
    /// Reference vertices for vertical and horizontal alignment, respectively.
    pub(super) guides: [Option<Point>; 2],
}

/// Segment projection in screen coordinates. Calls with coincident pixels treat the segment
/// as one point, which can happen when the canvas is zoomed out.
fn projection(pointer: Pos2, a: Pos2, b: Pos2) -> (f32, f32) {
    let delta = b - a;
    let t = if delta.length_sq() > 0. { ((pointer - a).dot(delta) / delta.length_sq()).clamp(0., 1.) } else { 0. };
    (t, pointer.distance(a + t * delta))
}

/// A hit on a vertex takes priority over the nearest edge.
pub(super) fn handle(pointer: Pos2, camera: &Camera, rect: Rect, drawing: &Drawing) -> Option<Handle> {
    let vertex = drawing
        .points
        .iter()
        .enumerate()
        .map(|(index, point)| (index, camera.screen(*point, rect).distance(pointer)))
        .filter(|(_, distance)| *distance <= VERTEX_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    vertex
        .map(|(index, _)| Handle::Vertex(index))
        .or_else(|| edge_distances(pointer, camera, rect, drawing).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(index, _)| Handle::Edge(index)))
}

/// Enumerate edge hits, including the implicit closing edge. Use the visible stroke width
/// with a minimum target radius for thin lines.
pub(super) fn edge_distances<'a>(pointer: Pos2, camera: &'a Camera, rect: Rect, drawing: &'a Drawing) -> impl Iterator<Item = (usize, f32)> + 'a {
    let tolerance =
        drawing.stroke_nm.map_or(6., |width| (width as f32 * camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32 / 2.).max(6.));
    drawing
        .points
        .iter()
        .zip(drawing.points.iter().cycle().skip(1))
        .take(drawing.points.len().saturating_sub(usize::from(!drawing.closed)))
        .enumerate()
        .filter_map(move |(index, (a, b))| {
            let (_, distance) = projection(pointer, camera.screen(*a, rect), camera.screen(*b, rect));
            (distance <= tolerance).then_some((index, distance))
        })
}

/// A body hit is meaningful only for closed paths. Use even-odd crossings so a concave
/// polygon's notches do not become drag targets.
pub(super) fn contains(drawing: &Drawing, point: Point) -> bool {
    if !drawing.closed {
        return false;
    }
    drawing
        .points
        .iter()
        .zip(drawing.points.iter().cycle().skip(1))
        .filter(|(a, b)| {
            let crosses = (a.y_nm > point.y_nm) != (b.y_nm > point.y_nm);
            crosses && {
                let t = (point.y_nm as f64 - a.y_nm as f64) / (b.y_nm as f64 - a.y_nm as f64);
                (point.x_nm as f64) < a.x_nm as f64 + (b.x_nm as f64 - a.x_nm as f64) * t
            }
        })
        .count()
        % 2
        == 1
}

/// Align either axis to adjacent reference vertices near the pointer. If both alignments
/// would collapse onto a reference vertex, retain just one viable axis where possible.
pub(super) fn align(pointer: Pos2, point: Point, references: impl IntoIterator<Item = Point>, camera: &Camera, rect: Rect) -> Snap {
    let mut x: Option<(f32, Point)> = None;
    let mut y: Option<(f32, Point)> = None;
    for reference in references {
        let distance = (camera.screen(reference, rect) - pointer).abs();
        if distance.x <= ALIGN_RADIUS && x.is_none_or(|(nearest, _)| distance.x < nearest) {
            x = Some((distance.x, reference));
        }
        if distance.y <= ALIGN_RADIUS && y.is_none_or(|(nearest, _)| distance.y < nearest) {
            y = Some((distance.y, reference));
        }
    }
    let mut adjusted = Point { x_nm: x.map_or(point.x_nm, |(_, r)| r.x_nm), y_nm: y.map_or(point.y_nm, |(_, r)| r.y_nm) };
    if x.is_some_and(|(_, r)| r == adjusted) || y.is_some_and(|(_, r)| r == adjusted) {
        if let Some((dx, _)) = x
            && y.is_some_and(|(dy, _)| dx <= dy)
            && point.y_nm != adjusted.y_nm
        {
            adjusted.y_nm = point.y_nm;
            y = None;
        } else if point.x_nm != adjusted.x_nm {
            adjusted.x_nm = point.x_nm;
            x = None;
        } else if point.y_nm != adjusted.y_nm {
            adjusted.y_nm = point.y_nm;
            y = None;
        } else {
            x = None;
            y = None;
        }
    }
    Snap { point: adjusted, guides: [x.map(|(_, r)| r), y.map(|(_, r)| r)] }
}

/// Move both endpoints by the same perpendicular offset, keeping the edge parallel and
/// leaving all other vertices fixed. Round once to integer nanometres.
pub(super) fn offset_edge(original: &[Point], index: usize, dx: i64, dy: i64) -> Vec<Point> {
    let next = (index + 1) % original.len();
    let x = (original[next].x_nm - original[index].x_nm) as f64;
    let y = (original[next].y_nm - original[index].y_nm) as f64;
    let scale = (dy as f64 * x - dx as f64 * y) / (x * x + y * y);
    let dx = (-y * scale).round() as i64;
    let dy = (x * scale).round() as i64;
    let mut points = original.to_vec();
    for i in [index, next] {
        points[i] = Point { x_nm: original[i].x_nm.saturating_add(dx), y_nm: original[i].y_nm.saturating_add(dy) };
    }
    points
}

/// Split at the nearest interior integer point along the edge. Most edges have an exact
/// lattice point there. If there is none, use the nearest nanometre to the projection.
pub(super) fn insertion(pointer: Pos2, camera: &Camera, rect: Rect, points: &[Point], index: usize) -> Option<Point> {
    let a = points[index];
    let b = points[(index + 1) % points.len()];
    let (t, _) = projection(pointer, camera.screen(a, rect), camera.screen(b, rect));
    let dx = b.x_nm - a.x_nm;
    let dy = b.y_nm - a.y_nm;
    let (mut steps, mut remainder) = (dx.abs(), dy.abs());
    while remainder != 0 {
        (steps, remainder) = (remainder, steps % remainder);
    }
    let interpolate = |d| {
        if steps > 1 {
            let step = (f64::from(t) * steps as f64).round() as i64;
            (d / steps) * step.clamp(1, steps - 1)
        } else {
            (f64::from(t) * d as f64).round() as i64
        }
    };
    let point = Point { x_nm: a.x_nm + interpolate(dx), y_nm: a.y_nm + interpolate(dy) };
    (point != a && point != b).then_some(point)
}
