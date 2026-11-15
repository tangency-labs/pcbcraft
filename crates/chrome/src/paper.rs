//! The schematic backdrop: the canvas around the sheet, then the paper with a soft shadow, its
//! edge, the grid, and a plain border inside it.
//!
//! Everything follows the chosen theme: the paper is white in the light theme and a darker
//! drawing surface in the dark theme, and the canvas around it keeps the theme's canvas colour
//! wherever the drawing lies. The paper outline marks the export area; it is not an editing
//! boundary. Every shape is clipped to the painted area, so the same shapes serve the canvas and
//! the ribbon margin that extends it.

use egui::{Color32, Pos2, Rect, Shape, Stroke, pos2};

use crate::{Tokens, grid, theme};

/// A sheet's paper in screen coordinates: its outer edge and the plain border inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    pub sheet: Rect,
    pub border: Rect,
}

/// Where the grid's zero lands on screen, the distance in points between grid points, the grid
/// opacity in percent, and whether the grid continues on the canvas outside the paper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub origin: Pos2,
    pub step: f32,
    pub opacity: u8,
    pub outside_paper: bool,
}

/// The backdrop shapes over `area` in the current palette. Without paper, the grid covers the
/// whole area, as on a sheet that predates paper. With paper, the grid covers the paper, and the
/// canvas around it too when [`Grid::outside_paper`] is set. The canvas fill never depends on
/// where the drawing lies.
pub fn backdrop(area: Rect, paper: Option<Paper>, grid: Grid) -> Vec<Shape> {
    let t = Tokens::get();
    let mut shapes = vec![Shape::rect_filled(area, 0, t.canvas)];
    let Some(paper) = paper else {
        if let Some(color) = grid::color(t.canvas_grid, grid.opacity) {
            shapes.extend(grid::shapes(area, grid.origin, grid.step, color, grid::Style::Dots));
        }
        return shapes;
    };
    let (under, over) = layers(area, paper);
    shapes.extend(under);
    let gridded = if grid.outside_paper { area } else { paper.sheet.intersect(area) };
    if gridded.is_positive()
        && let Some(color) = grid::color(t.canvas_grid, grid.opacity)
    {
        shapes.extend(grid::shapes(gridded, grid.origin, grid.step, color, grid::Style::Dots));
    }
    shapes.extend(over);
    shapes
}

/// The paper alone over `area`, without the canvas around it or a grid: for previews such as
/// Home thumbnails.
pub fn sheet(area: Rect, paper: Paper) -> Vec<Shape> {
    let (mut shapes, over) = layers(area, paper);
    shapes.extend(over);
    shapes
}

/// The paper's shadow and fill, then the border and edge that go over the grid, all clipped
/// to `area`.
fn layers(area: Rect, paper: Paper) -> (Vec<Shape>, Vec<Shape>) {
    let t = Tokens::get();
    let mut under = Vec::new();
    let shadow = paper.sheet.translate(theme::SCHEMATIC_PAPER_SHADOW).intersect(area);
    if shadow.is_positive() {
        under.push(Shape::rect_filled(shadow, 0, Color32::from_black_alpha(theme::SCHEMATIC_PAPER_SHADOW_ALPHA)));
    }
    let on_paper = paper.sheet.intersect(area);
    if on_paper.is_positive() {
        under.push(Shape::rect_filled(on_paper, 0, t.schematic_paper));
    }
    let mut over = outline(paper.border, area, Stroke::new(theme::SCHEMATIC_PAPER_BORDER, t.canvas_ink));
    over.extend(outline(paper.sheet, area, Stroke::new(1.0, t.border)));
    (under, over)
}

/// The four edges of `rect`, each clipped to `area`; edges outside it are left out.
fn outline(rect: Rect, area: Rect, stroke: Stroke) -> Vec<Shape> {
    let mut edges = Vec::new();
    for y in [rect.top(), rect.bottom()] {
        let (left, right) = (rect.left().max(area.left()), rect.right().min(area.right()));
        if (area.top()..=area.bottom()).contains(&y) && left < right {
            edges.push(Shape::line_segment([pos2(left, y), pos2(right, y)], stroke));
        }
    }
    for x in [rect.left(), rect.right()] {
        let (top, bottom) = (rect.top().max(area.top()), rect.bottom().min(area.bottom()));
        if (area.left()..=area.right()).contains(&x) && top < bottom {
            edges.push(Shape::line_segment([pos2(x, top), pos2(x, bottom)], stroke));
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    /// Collect every point a backdrop's shapes reach, to check clipping.
    fn points(shapes: &[Shape]) -> Vec<Pos2> {
        shapes
            .iter()
            .flat_map(|shape| match shape {
                Shape::LineSegment { points, .. } => points.to_vec(),
                Shape::Rect(rect) => vec![rect.rect.min, rect.rect.max],
                Shape::Circle(circle) => vec![circle.center],
                Shape::Vec(_) => panic!("nested shapes"),
                _ => Vec::new(),
            })
            .collect()
    }

    /// The centres of a backdrop's grid dots.
    fn dots(shapes: &[Shape]) -> Vec<Pos2> {
        shapes.iter().filter_map(|shape| if let Shape::Circle(dot) = shape { Some(dot.center) } else { None }).collect()
    }

    #[test]
    fn paper_backdrop_stays_inside_its_area_and_follows_the_grid_preference() {
        let area = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let grid = Grid { origin: Pos2::ZERO, step: 10.0, opacity: 100, outside_paper: false };
        let sheet = Rect::from_min_size(pos2(-50.0, 100.0), vec2(300.0, 400.0));
        let paper = Paper { sheet, border: sheet.shrink(5.0) };
        for dark in [true, false] {
            theme::apply(&egui::Context::default(), dark);
            let t = Tokens::get();
            let shapes = backdrop(area, Some(paper), grid);
            let Shape::Rect(surround) = &shapes[0] else { panic!("canvas first") };
            assert_eq!(surround.fill, t.canvas, "the canvas around the paper keeps the theme's colour");
            assert!(shapes.iter().any(|shape| matches!(shape, Shape::Rect(rect) if rect.fill == t.schematic_paper)));
            assert!(points(&shapes).iter().all(|p| area.expand(0.01).contains(*p)));
            let on_paper = dots(&shapes);
            assert!(!on_paper.is_empty());
            assert!(on_paper.iter().all(|dot| sheet.contains(*dot)), "grid dots only on the paper when outside is off");
            let everywhere = dots(&backdrop(area, Some(paper), Grid { outside_paper: true, ..grid }));
            assert!(everywhere.iter().any(|dot| !sheet.contains(*dot)), "the grid continues outside the paper when on");
            assert_eq!(everywhere.len(), dots(&backdrop(area, None, grid)).len(), "the same grid as a sheet without paper");
            let off = backdrop(area, Some(Paper { sheet: sheet.translate(vec2(1000.0, 0.0)), border: sheet }), grid);
            assert!(points(&off).iter().all(|p| area.expand(0.01).contains(*p)));
        }
    }
}
