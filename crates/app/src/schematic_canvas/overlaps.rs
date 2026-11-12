//! View-only warnings over drawing bounds, excluding pins, instance fields and wire connections.
use super::*;
use std::collections::BTreeMap;

const CELL: f32 = 16.;
const MAX_CELLS: i64 = 256;

#[derive(Clone, Debug)]
struct Cached {
    project: Project,
    revision: u64,
    step: f32,
    pixels_per_point: f32,
    index: Arc<Index>,
}

/// One view's derived body warnings; dropping or resetting the view releases its snapshot.
#[derive(Debug, Default)]
pub(crate) struct Cache {
    cached: Option<Cached>,
}

#[derive(Debug)]
pub(super) struct Index {
    entries: Vec<(String, Rect)>,
    cells: BTreeMap<(i32, i32), Vec<usize>>,
    large: Vec<usize>,
    overlaps: Vec<usize>,
    groups: Vec<Vec<usize>>,
}

impl Cache {
    /// Reuse immutable sheet geometry between frames, retaining the old snapshot to prevent
    /// pointer reuse on reopen. Revision and sheet identity guard references and projections;
    /// zoom and display scale invalidate the clamped, pixel-rounded drawing text extents.
    pub(super) fn get(&mut self, project: &Project, revision: u64, painter: &Painter, step: f32) -> Arc<Index> {
        if let Some(cache) = &self.cached
            && cache.revision == revision
            && cache.step == step
            && cache.pixels_per_point == painter.pixels_per_point()
            && cache.project.sheet_target() == project.sheet_target()
            && std::ptr::eq(cache.project.symbols(), project.symbols())
        {
            return cache.index.clone();
        }
        let entries = project
            .symbols()
            .iter()
            .filter_map(|symbol| bounds(painter, symbol.unit_definition(), symbol.turns(), symbol.position, step).map(|bounds| (symbol.id(), bounds)))
            .collect();
        let index = Arc::new(Index::new(entries));
        self.cached = Some(Cached { project: project.clone(), revision, step, pixels_per_point: painter.pixels_per_point(), index: index.clone() });
        index
    }
}

/// Convert drawing-only bounds to world grid units. Thin native strokes get a small extent;
/// an empty drawing has no body to warn about. Native text stays upright after rotation and
/// uses the rendered galley's full ink extent at this zoom. Fields and pin leads are excluded.
fn bounds(painter: &Painter, unit: &pcbcraft_engine::schematic::SymbolUnit, turns: u8, at: Point, step: f32) -> Option<Rect> {
    let drawing = drawing_bounds(unit);
    if !drawing.is_finite() {
        return None;
    }
    let padding = vec2((0.05 - drawing.width()).max(0.) / 2., (0.05 - drawing.height()).max(0.) / 2.);
    let mut drawing = turn_rect(drawing.expand2(padding), turns);
    if let Some(font) = drawing_text_font(step) {
        for shape in &unit.drawing {
            if let Shape::Text(anchor, text, align) = shape {
                let (x, y) = grid(*anchor);
                let (x, y) = turn(x, y, turns);
                let galley = painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
                let origin = text_anchor(*align, turns).anchor_size(pos2(x * step, y * step), galley.size()).min;
                let ink = galley.mesh_bounds.translate(origin.to_vec2());
                drawing = drawing.union(Rect::from_min_max(ink.min / step, ink.max / step));
            }
        }
    }
    Some(drawing.translate(vec2(at.x_nm as f32 / GRID, at.y_nm as f32 / GRID)))
}

/// Bound the cell expansion for large custom drawings; oversized bounds use the fallback list.
fn cells(bounds: Rect) -> Option<(i32, i32, i32, i32)> {
    let (left, top, right, bottom) = (
        (bounds.left() / CELL).floor() as i32,
        (bounds.top() / CELL).floor() as i32,
        (bounds.right() / CELL).floor() as i32,
        (bounds.bottom() / CELL).floor() as i32,
    );
    (i64::from(right) - i64::from(left) + 1)
        .checked_mul(i64::from(bottom) - i64::from(top) + 1)
        .is_some_and(|count| count <= MAX_CELLS)
        .then_some((left, top, right, bottom))
}

impl Index {
    /// Index drawing bounds once, then retain only whether each body has another intersecting
    /// body. Dense stacks stop at their first neighbor rather than enumerating every pair.
    fn new(entries: Vec<(String, Rect)>) -> Self {
        let mut index = Self { entries, cells: BTreeMap::new(), large: Vec::new(), overlaps: Vec::new(), groups: Vec::new() };
        for (i, (_, bounds)) in index.entries.iter().enumerate() {
            if let Some((left, top, right, bottom)) = cells(*bounds) {
                for x in left..=right {
                    for y in top..=bottom {
                        index.cells.entry((x, y)).or_default().push(i);
                    }
                }
            } else {
                index.large.push(i);
            }
        }
        let mut parents: Vec<_> = (0..index.entries.len()).collect();
        for (i, (_, bounds)) in index.entries.iter().enumerate() {
            if let Some(other) = index.neighbor(*bounds, Some(i)) {
                let a = root(&mut parents, i);
                let b = root(&mut parents, other);
                parents[a] = b;
                index.overlaps.push(i);
            }
        }
        let mut groups = BTreeMap::<usize, Vec<usize>>::new();
        for &i in &index.overlaps {
            groups.entry(root(&mut parents, i)).or_default().push(i);
        }
        index.groups = groups.into_values().collect();
        index
    }

    /// Find an actual positive-area bounds intersection, excluding self and edge-only contact.
    fn any(&self, bounds: Rect, except: Option<usize>) -> bool {
        self.neighbor(bounds, except).is_some()
    }

    /// Retain one intersecting witness per body, keeping dense-stack work bounded.
    fn neighbor(&self, bounds: Rect, except: Option<usize>) -> Option<usize> {
        let intersects = |i: usize| Some(i) != except && self.entries[i].1.intersect(bounds).is_positive();
        let Some((left, top, right, bottom)) = cells(bounds) else {
            return (0..self.entries.len()).find(|&i| intersects(i));
        };
        self.large.iter().copied().find(|&i| intersects(i)).or_else(|| {
            (left..=right).find_map(|x| {
                (top..=bottom).find_map(|y| self.cells.get(&(x, y)).and_then(|candidates| candidates.iter().copied().find(|&i| intersects(i))))
            })
        })
    }

    /// Paint one small shared warning badge per witness group, leaving symbol ink unobscured.
    /// Warnings are presentation only and remain visible for placements made through MCP/reopen.
    pub(super) fn paint(&self, ui: &mut Ui, id: egui::Id, project: &Project, camera: &Camera, canvas: Rect) {
        if !canvas.is_positive() {
            return;
        }
        let origin = camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas);
        for group in &self.groups {
            let world = group.iter().fold(Rect::NOTHING, |bounds, &i| bounds.union(self.entries[i].1));
            let rect = Rect::from_min_max(origin + world.min.to_vec2() * camera.unit_step(), origin + world.max.to_vec2() * camera.unit_step());
            if !rect.intersects(canvas) {
                continue;
            }
            let response = pcbcraft_chrome::canvas_warning::badge(
                ui,
                id.with(("overlap", &self.entries[group[0]].0)),
                rect.right_top() + vec2(6., -9.),
                canvas,
                &group.len().to_string(),
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, "Overlapping symbols"));
            response.on_hover_ui(|ui| {
                let mut names: Vec<_> = group
                    .iter()
                    .take(8)
                    .filter_map(|&i| {
                        let symbol = project.symbols().iter().find(|symbol| symbol.id() == self.entries[i].0)?;
                        Some(symbol.display_reference(&project.reference(symbol)))
                    })
                    .collect();
                if group.len() > names.len() {
                    names.push(format!("and {} more", group.len() - names.len()));
                }
                ui.label(format!("{} overlapping symbols: {}.", group.len(), names.join(", ")));
                ui.label("Finish placement, then right-click → Select symbol.");
            });
        }
    }

    /// Warn beside a new placement without preventing its commit or changing connection rules.
    pub(super) fn preview(&self, ui: &mut Ui, camera: &Camera, canvas: Rect, unit: &pcbcraft_engine::schematic::SymbolUnit, turns: u8, at: Point) {
        if bounds(ui.painter(), unit, turns, at, camera.unit_step()).is_some_and(|bounds| self.any(bounds, None)) {
            pcbcraft_chrome::canvas_warning::badge(
                ui,
                ui.scope_id().with("overlap_preview"),
                camera.screen(at, canvas) + vec2(14., 18.),
                canvas,
                "Symbol overlap",
            );
        }
    }
}

/// Compress witness links so stacked symbols share a count without enumerating every pair.
fn root(parents: &mut [usize], mut i: usize) -> usize {
    while parents[i] != i {
        let next = parents[parents[i]];
        parents[i] = next;
        i = next;
    }
    i
}

#[cfg(test)]
mod tests;
