//! Canvas path editing composed into workspaces with their own commands and history.
use crate::canvas_camera::Camera;
use egui::{Pos2, Rect};
use pcbcraft_engine::schematic::{GRID_NM, Point};

const VERTEX_RADIUS: f32 = 8.;
const ALIGN_RADIUS: f32 = 6.;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Handle {
    Vertex(usize),
    Edge(usize),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Snap {
    pub(crate) point: Point,
    /// Reference vertices for vertical and horizontal alignment, respectively.
    pub(crate) guides: [Option<Point>; 2],
}

/// Segment projection in screen coordinates. Calls with coincident pixels treat the segment
/// as one point, which can happen when the canvas is zoomed out.
fn projection(pointer: Pos2, a: Pos2, b: Pos2) -> (f32, f32) {
    let delta = b - a;
    let t = if delta.length_sq() > 0. { ((pointer - a).dot(delta) / delta.length_sq()).clamp(0., 1.) } else { 0. };
    (t, pointer.distance(a + t * delta))
}

/// A borrowed path and its physical hit width. Object identity and storage stay with the host.
#[derive(Clone, Copy)]
pub(crate) struct Path<'a> {
    pub(crate) points: &'a [Point],
    pub(crate) closed: bool,
    pub(crate) stroke_nm: Option<i64>,
}

/// The same world-to-screen camera used by the host for picking, painting, and snapping.
#[derive(Clone, Copy)]
pub(crate) struct Canvas<'a> {
    pub(crate) camera: &'a Camera,
    pub(crate) rect: Rect,
}

impl Canvas<'_> {
    fn screen(self, point: Point) -> Pos2 {
        self.camera.screen(point, self.rect)
    }

    /// Find an exact external vertex before relying on alignment or the grid.
    pub(crate) fn nearby_vertex(self, pointer: Pos2, points: impl IntoIterator<Item = Point>) -> Option<Point> {
        points
            .into_iter()
            .filter_map(|point| {
                let distance = self.screen(point).distance(pointer);
                (distance <= VERTEX_RADIUS).then_some((distance, point))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, point)| point)
    }

    /// Vertices get priority over edges so the same cursor never requests both edits.
    pub(crate) fn handle(self, pointer: Pos2, path: Path<'_>) -> Option<Handle> {
        path.points
            .iter()
            .enumerate()
            .map(|(index, point)| (index, self.screen(*point).distance(pointer)))
            .filter(|(_, distance)| *distance <= VERTEX_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| Handle::Vertex(index))
            .or_else(|| self.edges(pointer, path).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(index, _)| Handle::Edge(index)))
    }

    /// Use the visible physical stroke, with a minimum target for hairlines.
    pub(crate) fn hit_edge(self, pointer: Pos2, path: Path<'_>) -> bool {
        self.edges(pointer, path).next().is_some()
    }

    /// A selected path can be grabbed from its interior without making interiors pick targets.
    pub(crate) fn hit_body(self, pointer: Pos2, path: Path<'_>) -> bool {
        contains(path, self.camera.unsnapped(pointer, self.rect))
    }

    /// Offer exact contact first, then grid and optional axis alignment at the same cursor.
    pub(crate) fn snap(self, pointer: Pos2, references: impl IntoIterator<Item = Point>, contact: Option<Point>, bypass: bool) -> Snap {
        let point = contact.unwrap_or_else(|| self.camera.snap(pointer, self.rect));
        if bypass || contact.is_some() { Snap { point, guides: [None; 2] } } else { align(self, pointer, point, references) }
    }

    /// Paint alignment in the same projection used to choose the snap.
    pub(crate) fn paint_guides(self, painter: &egui::Painter, snap: Snap) {
        for reference in snap.guides.into_iter().flatten() {
            pcbcraft_chrome::board::drawing_alignment(painter, [self.screen(reference), self.screen(snap.point)]);
        }
    }

    /// Enumerate edge hits including the implicit closing edge of a polygon.
    fn edges<'a>(self, pointer: Pos2, path: Path<'a>) -> impl Iterator<Item = (usize, f32)> + 'a
    where
        Self: 'a,
    {
        let tolerance = path.stroke_nm.map_or(6., |width| (width as f32 * self.camera.unit_step() / GRID_NM as f32 / 2.).max(6.));
        path.points
            .iter()
            .zip(path.points.iter().cycle().skip(1))
            .take(path.points.len().saturating_sub(usize::from(!path.closed)))
            .enumerate()
            .filter_map(move |(index, (a, b))| {
                let (_, distance) = projection(pointer, self.screen(*a), self.screen(*b));
                (distance <= tolerance).then_some((index, distance))
            })
    }
}

/// A body hit is meaningful only for closed paths. Use even-odd crossings so a concave
/// polygon's notches do not become drag targets.
fn contains(path: Path<'_>, point: Point) -> bool {
    if !path.closed {
        return false;
    }
    path.points
        .iter()
        .zip(path.points.iter().cycle().skip(1))
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
fn align(canvas: Canvas<'_>, pointer: Pos2, point: Point, references: impl IntoIterator<Item = Point>) -> Snap {
    let mut x: Option<(f32, Point)> = None;
    let mut y: Option<(f32, Point)> = None;
    for reference in references {
        let distance = (canvas.screen(reference) - pointer).abs();
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
fn offset_edge(original: &[Point], index: usize, dx: i64, dy: i64) -> Vec<Point> {
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
fn insertion(canvas: Canvas<'_>, pointer: Pos2, points: &[Point], index: usize) -> Option<Point> {
    let a = points[index];
    let b = points[(index + 1) % points.len()];
    let (t, _) = projection(pointer, canvas.screen(a), canvas.screen(b));
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

/// One disposable drag. The host captures the object and revision and applies the finished
/// vertices through its existing undoable transaction.
#[derive(Clone, Debug)]
pub(crate) struct Drag {
    original: Vec<Point>,
    pub(crate) points: Vec<Point>,
    closed: bool,
    handle: Option<Handle>,
    anchor: Point,
    pub(crate) alignment: Option<Snap>,
}

impl Drag {
    /// Open-path edge drags and handle-free drags move the whole path.
    pub(crate) fn new(canvas: Canvas<'_>, path: Path<'_>, pointer: Pos2, handle: Option<Handle>) -> Self {
        let handle = handle.filter(|handle| path.closed || matches!(handle, Handle::Vertex(_)));
        Self {
            original: path.points.to_vec(),
            points: path.points.to_vec(),
            closed: path.closed,
            handle,
            anchor: canvas.camera.snap(pointer, canvas.rect),
            alignment: None,
        }
    }

    /// Recompute from the captured path to avoid accumulating rounding as the pointer moves.
    pub(crate) fn update(&mut self, canvas: Canvas<'_>, pointer: Pos2, contact: Option<Point>, bypass: bool) {
        let references = if let Some(Handle::Vertex(index)) = self.handle {
            let n = self.original.len();
            [
                (self.closed || index > 0).then(|| self.original[(index + n - 1) % n]),
                (self.closed || index + 1 < n).then(|| self.original[(index + 1) % n]),
            ]
        } else {
            [None; 2]
        };
        let snap = canvas.snap(pointer, references.into_iter().flatten(), contact, bypass);
        let dx = snap.point.x_nm.saturating_sub(self.anchor.x_nm);
        let dy = snap.point.y_nm.saturating_sub(self.anchor.y_nm);
        self.alignment = None;
        match self.handle {
            Some(Handle::Vertex(index)) => {
                self.points[index] = snap.point;
                self.alignment = Some(snap);
            }
            Some(Handle::Edge(index)) => self.points = offset_edge(&self.original, index, dx, dy),
            None => {
                for (point, from) in self.points.iter_mut().zip(&self.original) {
                    *point = Point { x_nm: from.x_nm.saturating_add(dx), y_nm: from.y_nm.saturating_add(dy) };
                }
            }
        }
    }
}

/// A captured context-menu candidate. A disabled row shows a geometric or count limit.
#[derive(Clone, Debug)]
pub(crate) struct VertexAction {
    label: &'static str,
    points: Option<Vec<Point>>,
}

impl VertexAction {
    /// Split the addressed edge or remove the addressed corner. The host supplies its model limits.
    pub(crate) fn at(canvas: Canvas<'_>, pointer: Pos2, path: Path<'_>, max_points: usize, valid: impl FnOnce(&[Point]) -> bool) -> Option<Self> {
        let mut points = path.points.to_vec();
        let (label, possible) = match canvas.handle(pointer, path)? {
            Handle::Vertex(index) => {
                points.remove(index);
                ("Remove vertex", points.len() >= if path.closed { 3 } else { 2 })
            }
            Handle::Edge(index) => {
                let point = insertion(canvas, pointer, &points, index);
                (
                    "Add vertex",
                    points.len() < max_points
                        && point.is_some_and(|point| {
                            points.insert(index + 1, point);
                            true
                        }),
                )
            }
        };
        let points = possible.then_some(points).filter(|points| valid(points));
        Some(Self { label, points })
    }

    /// Use the same full-width menu row in any canvas.
    pub(crate) fn show(&self, ui: &mut egui::Ui) -> Option<Vec<Point>> {
        if pcbcraft_chrome::menu::MenuItem::new(self.label).enabled(self.points.is_some()).show(ui) { self.points.clone() } else { None }
    }
}
