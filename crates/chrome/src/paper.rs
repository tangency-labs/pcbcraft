//! The schematic backdrop: the canvas around the sheet, then white paper with a soft shadow,
//! its edge, the grid on the paper only, and a plain border inside it.
//!
//! The paper keeps the light palette in either theme; only the canvas around it and the paper's
//! edge follow the chosen theme. Every shape is clipped to the painted area, so the same shapes
//! serve the canvas and the ribbon margin that extends it.

use egui::{Color32, Pos2, Rect, Shape, Stroke, pos2};

use crate::{Tokens, grid, theme};

/// A sheet's paper in screen coordinates: its outer edge and the plain border inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    pub sheet: Rect,
    pub border: Rect,
}

/// Where the grid's zero lands on screen, the distance in points between grid points, and the
/// grid opacity in percent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub origin: Pos2,
    pub step: f32,
    pub opacity: u8,
}

/// The backdrop shapes over `area`. Without paper, the grid covers the whole area in the current
/// palette, as on a sheet that predates paper. With paper, the area outside it is plain canvas in
/// the chosen theme, unless `drawing_off_paper` says part of the drawing lies outside the paper:
/// that drawing keeps the paper's light palette, so the canvas around the paper turns light too
/// and the drawing stays readable in the dark theme.
pub fn backdrop(area: Rect, paper: Option<Paper>, grid: Grid, drawing_off_paper: bool) -> Vec<Shape> {
    let surround = if paper.is_some() && drawing_off_paper { Tokens::light().canvas } else { Tokens::chrome().canvas };
    let mut shapes = vec![Shape::rect_filled(area, 0, surround)];
    let Some(paper) = paper else {
        if let Some(color) = grid::color(Tokens::get().canvas_grid, grid.opacity) {
            shapes.extend(grid::shapes(area, grid.origin, grid.step, color, grid::Style::Dots));
        }
        return shapes;
    };
    let (under, over) = layers(area, paper);
    shapes.extend(under);
    let on_paper = paper.sheet.intersect(area);
    if on_paper.is_positive()
        && let Some(color) = grid::color(Tokens::light().canvas_grid, grid.opacity)
    {
        shapes.extend(grid::shapes(on_paper, grid.origin, grid.step, color, grid::Style::Dots));
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
    let mut under = Vec::new();
    let shadow = paper.sheet.translate(theme::SCHEMATIC_PAPER_SHADOW).intersect(area);
    if shadow.is_positive() {
        under.push(Shape::rect_filled(shadow, 0, Color32::from_black_alpha(theme::SCHEMATIC_PAPER_SHADOW_ALPHA)));
    }
    let on_paper = paper.sheet.intersect(area);
    if on_paper.is_positive() {
        under.push(Shape::rect_filled(on_paper, 0, theme::SCHEMATIC_PAPER));
    }
    let mut over = outline(paper.border, area, Stroke::new(theme::SCHEMATIC_PAPER_BORDER, Tokens::light().canvas_ink));
    over.extend(outline(paper.sheet, area, Stroke::new(1.0, Tokens::chrome().border)));
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

    #[test]
    fn paper_backdrop_stays_inside_its_area_and_grid_only_on_paper() {
        let area = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let grid = Grid { origin: Pos2::ZERO, step: 10.0, opacity: 100 };
        let sheet = Rect::from_min_size(pos2(-50.0, 100.0), vec2(300.0, 400.0));
        let paper = Paper { sheet, border: sheet.shrink(5.0) };
        let shapes = backdrop(area, Some(paper), grid, false);
        let Shape::Rect(surround) = &shapes[0] else { panic!("canvas first") };
        assert_eq!(surround.fill, Tokens::chrome().canvas);
        let Shape::Rect(spilled) = &backdrop(area, Some(paper), grid, true)[0] else { panic!("canvas first") };
        assert_eq!(spilled.fill, Tokens::light().canvas, "drawing off the paper keeps a light surround");
        assert!(points(&shapes).iter().all(|p| area.expand(0.01).contains(*p)));
        let dots: Vec<_> = shapes.iter().filter_map(|shape| if let Shape::Circle(dot) = shape { Some(dot.center) } else { None }).collect();
        assert!(!dots.is_empty());
        assert!(dots.iter().all(|dot| sheet.contains(*dot)), "grid dots only on the paper");
        let bare = backdrop(area, None, grid, true);
        assert!(bare.iter().filter(|shape| matches!(shape, Shape::Circle(_))).count() > dots.len(), "without paper the grid covers the whole area");
        let off = backdrop(area, Some(Paper { sheet: sheet.translate(vec2(1000.0, 0.0)), border: sheet }), grid, false);
        assert!(points(&off).iter().all(|p| area.expand(0.01).contains(*p)));
    }
}
