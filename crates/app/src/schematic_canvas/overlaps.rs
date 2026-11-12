//! View-only warnings over drawing bounds, excluding pins, instance fields and wire connections.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

const CELL: f32 = 16.;
const MAX_CELLS: i64 = 256;

#[derive(Clone, Debug)]
struct Cached {
    project: Project,
    revision: u64,
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
}

impl Cache {
    /// Reuse immutable sheet geometry between frames, retaining the old snapshot to prevent
    /// pointer reuse on reopen. Revision and sheet identity guard references and projections.
    pub(super) fn get(&mut self, project: &Project, revision: u64) -> Arc<Index> {
        if let Some(cache) = &self.cached
            && cache.revision == revision
            && cache.project.sheet_target() == project.sheet_target()
            && std::ptr::eq(cache.project.symbols(), project.symbols())
        {
            return cache.index.clone();
        }
        let entries = project
            .symbols()
            .iter()
            .filter_map(|symbol| bounds(symbol.unit_definition(), symbol.turns(), symbol.position).map(|bounds| (symbol.id(), bounds)))
            .collect();
        let index = Arc::new(Index::new(entries));
        self.cached = Some(Cached { project: project.clone(), revision, index: index.clone() });
        index
    }
}

/// Convert drawing-only bounds to world grid units. Thin native strokes get a small extent;
/// an empty drawing has no body to warn about. Field text and integrated pin leads are excluded.
fn bounds(unit: &pcbcraft_engine::schematic::SymbolUnit, turns: u8, at: Point) -> Option<Rect> {
    let drawing = drawing_bounds(unit);
    if !drawing.is_finite() {
        return None;
    }
    let padding = vec2((0.05 - drawing.width()).max(0.) / 2., (0.05 - drawing.height()).max(0.) / 2.);
    Some(turn_rect(drawing.expand2(padding), turns).translate(vec2(at.x_nm as f32 / GRID, at.y_nm as f32 / GRID)))
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
        let mut index = Self { entries, cells: BTreeMap::new(), large: Vec::new(), overlaps: Vec::new() };
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
        index.overlaps = index.entries.iter().enumerate().filter_map(|(i, (_, bounds))| index.any(*bounds, Some(i)).then_some(i)).collect();
        index
    }

    /// Find an actual positive-area bounds intersection, excluding self and edge-only contact.
    fn any(&self, bounds: Rect, except: Option<usize>) -> bool {
        let intersects = |i: usize| Some(i) != except && self.entries[i].1.intersect(bounds).is_positive();
        let Some((left, top, right, bottom)) = cells(bounds) else {
            return (0..self.entries.len()).any(intersects);
        };
        self.large.iter().copied().any(intersects)
            || (left..=right)
                .any(|x| (top..=bottom).any(|y| self.cells.get(&(x, y)).is_some_and(|candidates| candidates.iter().copied().any(intersects))))
    }

    /// Paint a persistent outline and shared warning marker, coalescing identical stacked bounds.
    /// Warnings are presentation only and remain visible for placements made through MCP/reopen.
    pub(super) fn paint(&self, ui: &Ui, id: egui::Id, painter: &Painter, camera: &Camera, canvas: Rect) {
        if !canvas.is_positive() {
            return;
        }
        let origin = camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas);
        let inset = canvas.shrink2(vec2(10f32.min(canvas.width() / 2.), 10f32.min(canvas.height() / 2.)));
        let mut painted = BTreeSet::new();
        for &i in &self.overlaps {
            let (symbol, world) = &self.entries[i];
            let key = (world.min.x.to_bits(), world.min.y.to_bits(), world.max.x.to_bits(), world.max.y.to_bits());
            let rect = Rect::from_min_max(origin + world.min.to_vec2() * camera.unit_step(), origin + world.max.to_vec2() * camera.unit_step());
            if !rect.intersects(canvas) || !painted.insert(key) {
                continue;
            }
            painter.rect_stroke(rect.expand(2.), 1., Stroke::new(1., Tokens::get().error), egui::StrokeKind::Outside);
            let marker = (rect.right_top() + vec2(8., -8.)).clamp(inset.min, inset.max);
            pcbcraft_chrome::board::drc_marker(painter, marker, false);
            let response = ui.interact(Rect::from_center_size(marker, vec2(18., 18.)), id.with(("overlap", symbol)), Sense::hover());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, "Overlapping symbols"));
            response.on_hover_text("Symbol bodies overlap here. Finish placement, then right-click a symbol and use Select symbol to choose one.");
        }
    }

    /// Warn beside a new placement without preventing its commit or changing connection rules.
    pub(super) fn preview(
        &self,
        painter: &Painter,
        camera: &Camera,
        canvas: Rect,
        unit: &pcbcraft_engine::schematic::SymbolUnit,
        turns: u8,
        at: Point,
    ) {
        if bounds(unit, turns, at).is_some_and(|bounds| self.any(bounds, None)) {
            let color = Tokens::get().error;
            let label = painter.layout_no_wrap("Symbol overlap".into(), FontId::proportional(Tokens::get().font_small), color);
            let size = vec2(label.size().x + 28., label.size().y.max(18.));
            let position = (camera.screen(at, canvas) + vec2(14., 18.)).clamp(canvas.min, (canvas.max - size).max(canvas.min));
            pcbcraft_chrome::board::drc_marker(painter, position + vec2(8., size.y / 2.), false);
            painter.galley(position + vec2(22., (size.y - label.size().y) / 2.), label, color);
        }
    }
}

#[cfg(test)]
mod tests;
