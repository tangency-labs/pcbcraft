//! The schematic backdrop: the canvas around the sheet, then the paper with a soft shadow, its
//! edge, the grid, and the frame drawn on it: a border, zone markers and a title block.
//!
//! Paper and its surround share one canvas colour by default. Optional shading or an opaque
//! custom paper colour changes only the paper fill. The paper outline marks the export area;
//! it is not an editing boundary. Frame lines are clipped to the painted area, so they serve the canvas and
//! the ribbon margin that extends it.

use std::ops::Range;

use egui::{Align, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2};

use crate::{Tokens, canvas_text, grid, theme};

/// Frame texts smaller than this many points are left out to avoid unreadable layout.
const MIN_TEXT_SIZE: f32 = 3.0;

/// Personal canvas and optional paper fill, resolved from the active theme and preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub canvas: Color32,
    /// None keeps the canvas uniform, including removal of the paper shadow.
    pub paper: Option<Color32>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { canvas: Tokens::get().canvas, paper: None }
    }
}

impl Appearance {
    /// The surface beneath on-paper artwork; a uniform page uses the canvas colour.
    pub fn fill(self) -> Color32 {
        self.paper.unwrap_or(self.canvas)
    }
}

/// A sheet's paper in screen coordinates, including its frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Paper {
    pub sheet: Rect,
    pub frame: Frame,
}

/// The border, zone markers and title block, in screen coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub lines: Vec<[Pos2; 2]>,
    pub texts: Vec<FrameText>,
}

/// One frame text: one line whose middle sits on `at`.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameText {
    pub at: Pos2,
    /// Font size in points.
    pub size: f32,
    /// Which part of the text sits on `at` horizontally.
    pub align: Align,
    /// Field labels and zone markers are quieter than values.
    pub label: bool,
    pub text: String,
    /// Byte ranges of `text` drawn in the unresolved-reference colour.
    pub unknown: Vec<Range<usize>>,
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

/// The backdrop shapes over `area` with resolved personal colours. Without paper, the grid covers the
/// whole area, as on a sheet that predates paper. With paper, the grid covers the paper, and the
/// canvas around it too when [`Grid::outside_paper`] is set. The canvas fill never depends on
/// where the drawing lies. `painter` lays out the frame texts.
pub fn backdrop(painter: &Painter, area: Rect, paper: Option<&Paper>, grid: Grid, appearance: Appearance) -> Vec<Shape> {
    let t = Tokens::get();
    let mut shapes = vec![Shape::rect_filled(area, 0, appearance.canvas)];
    let Some(paper) = paper else {
        if let Some(color) = grid::color(t.canvas_grid, grid.opacity) {
            shapes.extend(grid::shapes(area, grid.origin, grid.step, color, grid::Style::Dots));
        }
        return shapes;
    };
    let (under, over) = layers(painter, area, paper, appearance);
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
pub fn sheet(painter: &Painter, area: Rect, paper: &Paper, appearance: Appearance) -> Vec<Shape> {
    let (mut shapes, over) = layers(painter, area, paper, appearance);
    shapes.extend(over);
    shapes
}

/// The optional paper shadow and fill, then the frame and edge over the grid, clipped
/// to `area`. Uniform presentation omits the shadow and fills with the canvas colour.
fn layers(painter: &Painter, area: Rect, paper: &Paper, appearance: Appearance) -> (Vec<Shape>, Vec<Shape>) {
    let t = Tokens::get();
    let mut under = Vec::new();
    let shadow = paper.sheet.translate(theme::SCHEMATIC_PAPER_SHADOW).intersect(area);
    if appearance.paper.is_some() && shadow.is_positive() {
        under.push(Shape::rect_filled(shadow, 0, Color32::from_black_alpha(theme::SCHEMATIC_PAPER_SHADOW_ALPHA)));
    }
    let on_paper = paper.sheet.intersect(area);
    if on_paper.is_positive() {
        under.push(Shape::rect_filled(on_paper, 0, appearance.fill()));
    }
    let ink = Stroke::new(theme::SCHEMATIC_PAPER_BORDER, t.canvas_ink);
    let mut over: Vec<Shape> =
        paper.frame.lines.iter().filter_map(|&[a, b]| clip(a, b, area)).map(|points| Shape::line_segment(points, ink)).collect();
    for text in paper.frame.texts.iter().filter(|text| text.size >= MIN_TEXT_SIZE && !text.text.is_empty()) {
        let color = if text.label { t.text_dim } else { t.canvas_ink };
        let galley = canvas_text::layout_runs(painter, &runs(text, color), FontId::proportional(text.size));
        let bounds = Align2([text.align, Align::Center]).anchor_size(text.at, galley.size());
        if bounds.intersects(area) {
            over.push(Shape::galley(bounds.min, galley, color));
        }
    }
    let edge = Stroke::new(1.0, t.border);
    let [min, max] = [paper.sheet.min, paper.sheet.max];
    let corners = [min, pos2(max.x, min.y), max, pos2(min.x, max.y)];
    over.extend((0..4).filter_map(|i| clip(corners[i], corners[(i + 1) % 4], area)).map(|points| Shape::line_segment(points, edge)));
    (under, over)
}

/// Split a frame text into its plain and unresolved runs, with their colours.
fn runs(text: &FrameText, color: Color32) -> Vec<(&str, Color32)> {
    let mut out = Vec::new();
    let mut from = 0;
    for range in &text.unknown {
        if let (Some(plain), Some(unknown)) = (text.text.get(from..range.start), text.text.get(range.clone())) {
            out.push((plain, color));
            out.push((unknown, theme::SCHEMATIC_UNRESOLVED));
            from = range.end;
        }
    }
    out.push((text.text.get(from..).unwrap_or(""), color));
    out.retain(|(run, _)| !run.is_empty());
    out
}

/// The part of the segment from `a` to `b` inside `area` (Liang-Barsky), or `None` when it
/// misses the area.
fn clip(a: Pos2, b: Pos2, area: Rect) -> Option<[Pos2; 2]> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (p, q) in [(-d.x, a.x - area.left()), (d.x, area.right() - a.x), (-d.y, a.y - area.top()), (d.y, area.bottom() - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some([a + d * t0, a + d * t1])
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
        let border = sheet.shrink(5.0);
        let lines =
            vec![[border.left_top(), border.right_top()], [border.right_top(), border.right_bottom()], [border.left_bottom(), pos2(500.0, 700.0)]];
        let paper = Paper { sheet, frame: Frame { lines, texts: Vec::new() } };
        let ctx = egui::Context::default();
        let painter = Painter::new(ctx.clone(), egui::LayerId::background(), area);
        for dark in [true, false] {
            theme::apply(&ctx, dark);
            let t = Tokens::get();
            let appearance = Appearance::default();
            let shapes = backdrop(&painter, area, Some(&paper), grid, appearance);
            let Shape::Rect(surround) = &shapes[0] else { panic!("canvas first") };
            assert_eq!(surround.fill, t.canvas, "the canvas around the paper keeps the theme's colour");
            assert!(shapes.iter().any(|shape| matches!(shape, Shape::Rect(rect) if rect.rect == sheet.intersect(area) && rect.fill == t.canvas)));
            assert!(!shapes.iter().any(|shape| matches!(shape, Shape::Rect(rect) if rect.fill.a() < 255)), "uniform paper has no shadow");
            for color in [t.schematic_paper, Color32::from_rgb(50, 70, 90)] {
                let appearance = Appearance { paper: Some(color), ..appearance };
                let styled = backdrop(&painter, area, Some(&paper), grid, appearance);
                assert!(
                    styled
                        .iter()
                        .any(|shape| matches!(shape, Shape::Rect(rect) if rect.rect == sheet.intersect(area) && rect.fill == appearance.fill()))
                );
            }
            assert!(points(&shapes).iter().all(|p| area.expand(0.01).contains(*p)));
            let on_paper = dots(&shapes);
            assert!(!on_paper.is_empty());
            assert!(on_paper.iter().all(|dot| sheet.contains(*dot)), "grid dots only on the paper when outside is off");
            let everywhere = dots(&backdrop(&painter, area, Some(&paper), Grid { outside_paper: true, ..grid }, appearance));
            assert!(everywhere.iter().any(|dot| !sheet.contains(*dot)), "the grid continues outside the paper when on");
            assert_eq!(everywhere.len(), dots(&backdrop(&painter, area, None, grid, appearance)).len(), "the same grid as a sheet without paper");
            let off = backdrop(&painter, area, Some(&Paper { sheet: sheet.translate(vec2(1000.0, 0.0)), frame: paper.frame.clone() }), grid, appearance);
            assert!(points(&off).iter().all(|p| area.expand(0.01).contains(*p)));
        }
    }

    #[test]
    fn unresolved_runs_are_coloured_and_lines_are_clipped() {
        let text = FrameText {
            at: Pos2::ZERO,
            size: 10.0,
            align: Align::Min,
            label: false,
            text: "@project.Nope Ltd".into(),
            unknown: std::iter::once(0..13).collect(),
        };
        assert_eq!(runs(&text, Color32::BLACK), vec![("@project.Nope", theme::SCHEMATIC_UNRESOLVED), (" Ltd", Color32::BLACK)]);
        let plain = FrameText { unknown: Vec::new(), ..text };
        assert_eq!(runs(&plain, Color32::BLACK), vec![("@project.Nope Ltd", Color32::BLACK)]);
        let area = Rect::from_min_size(Pos2::ZERO, vec2(10.0, 10.0));
        assert_eq!(clip(pos2(-10.0, 5.0), pos2(20.0, 5.0), area), Some([pos2(0.0, 5.0), pos2(10.0, 5.0)]));
        assert_eq!(clip(pos2(-10.0, -10.0), pos2(5.0, 5.0), area), Some([pos2(0.0, 0.0), pos2(5.0, 5.0)]));
        assert_eq!(clip(pos2(-10.0, 20.0), pos2(20.0, 20.0), area), None);
        assert_eq!(clip(pos2(11.0, 0.0), pos2(20.0, 9.0), area), None);
    }
}
