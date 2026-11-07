//! A compact, read-only schematic view for Home. It reuses the canvas symbol painter and
//! engine wire paths; egui remains the renderer, with no second rasterizer or graphics device.

use super::*;

/// Bound cached geometry and per-frame paint work independently of the full project limits.
const MAX_POINTS: usize = 100_000;

#[derive(Debug)]
struct Symbol {
    placement: SymbolPlacement,
    reference: String,
    value: String,
}

/// Disposable geometry from a saved snapshot, with no session, selection, camera or undo state.
#[derive(Debug)]
pub(crate) struct Thumbnail {
    symbols: Vec<Symbol>,
    wires: Vec<Vec<Pos2>>,
    bounds: Rect,
}

impl Thumbnail {
    /// Prepare on a worker, keeping pinned definitions and exact authored routes. Oversized
    /// drawings yield no preview instead of silently truncating or monopolizing the UI thread.
    pub(crate) fn new(project: &Project) -> Option<Self> {
        let mut points = 0;
        let mut bounds = Rect::NOTHING;
        let mut symbols = Vec::new();
        for symbol in project.symbols() {
            points += symbol
                .definition
                .drawing
                .iter()
                .map(|shape| match shape {
                    Shape::Polyline(points) => points.len(),
                    _ => 4,
                })
                .sum::<usize>();
            if points > MAX_POINTS {
                return None;
            }
            bounds = bounds.union(grid_bounds(&symbol.definition, symbol.turns()).translate(grid_pos(symbol.position).to_vec2()));
            symbols.push(Symbol { placement: symbol.clone(), reference: project.reference(symbol), value: project.value(symbol).to_owned() });
        }
        let mut wires = Vec::new();
        for wire in project.wires() {
            let path = project.wire_path(wire);
            points += path.len();
            if points > MAX_POINTS {
                return None;
            }
            let path: Vec<_> = path.into_iter().map(grid_pos).collect();
            for point in &path {
                bounds = bounds.union(Rect::from_pos(*point));
            }
            wires.push(path);
        }
        Some(Self { symbols, wires, bounds })
    }

    /// An empty saved schematic has its own placeholder rather than pretending a load failed.
    pub(crate) fn is_empty(&self) -> bool {
        self.symbols.is_empty() && self.wires.is_empty()
    }

    /// Fit the saved geometry in the card and paint with the active theme. Canvas labels follow
    /// the existing zoom threshold; library strokes, rotations and wire corners use the same data.
    pub(crate) fn paint(&self, painter: &Painter, rect: Rect) {
        if self.is_empty() || !rect.is_positive() {
            return;
        }
        let t = Tokens::get();
        let target = rect.shrink(t.space_8);
        let bounds = self.bounds.expand(theme::SCHEMATIC_FIT_MARGIN / theme::SCHEMATIC_GRID_SPACING);
        let step = (target.width() / bounds.width()).min(target.height() / bounds.height()).min(theme::SCHEMATIC_PREVIEW_MAX_STEP);
        let screen = |point: Pos2| target.center() + (point - bounds.center()) * step;
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect));
        for path in &self.wires {
            painter
                .add(egui::Shape::line(path.iter().copied().map(screen).collect(), Stroke::new(theme::SCHEMATIC_PREVIEW_STROKE, t.schematic_wire)));
        }
        for symbol in &self.symbols {
            let placement = &symbol.placement;
            let label = (step >= theme::SCHEMATIC_MIN_GRID_SPACING).then_some((symbol.reference.as_str(), symbol.value.as_str()));
            paint_symbol(
                &painter,
                &placement.definition,
                screen(grid_pos(placement.position)),
                step,
                placement.turns(),
                Stroke::new(theme::SCHEMATIC_PREVIEW_STROKE, t.text),
                label,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::Session;
    use serde_json::json;

    /// Snapshot preparation preserves every authored route and rotated symbol without touching
    /// the project; an empty document remains a valid, distinguishable empty preview.
    #[test]
    fn thumbnail_uses_saved_geometry_without_editor_state() {
        let project = pcbcraft_engine::persistence::decode(include_bytes!("../../tests/fixtures/two-resistors.pc")).unwrap();
        let preview = Thumbnail::new(&project).unwrap();
        assert_eq!(preview.symbols.len(), project.symbols().len());
        assert_eq!(preview.wires[0], project.wire_path(&project.wires()[0]).into_iter().map(grid_pos).collect::<Vec<_>>());
        assert!(preview.bounds.is_finite());
        let mut session = Session::default();
        session.apply("project.new", &json!({"name":"Empty"})).unwrap();
        assert!(Thumbnail::new(session.project().unwrap()).unwrap().is_empty());
    }
}
