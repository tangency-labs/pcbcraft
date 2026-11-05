//! Theme-colored File and app bar glyphs. Asset sources are listed in ATTRIBUTION.md.

use egui::{Painter, Pos2, Rect, Shape, Stroke, StrokeKind, vec2};

use crate::Tokens;

/// Glyphs shared by File rows and app bar actions.
#[derive(Clone, Copy, Debug)]
pub enum MenuIcon {
    Home,
    New,
    Open,
    Save,
    Download,
    Rename,
    Close,
    Undo,
    Redo,
    Settings,
    Help,
    Bug,
    /// Preferences pages and the settings search field.
    Appearance,
    Navigation,
    Keyboard,
    Search,
    Assistant,
}

impl MenuIcon {
    /// Paint a 16-point glyph. The caller selects brighter ink for active controls or hovered rows
    /// and sets the painter opacity when the action is disabled.
    pub(super) fn paint(self, painter: &Painter, center: Pos2, t: Tokens, hot: bool) {
        self.paint_sized(painter, center, t.space_16, t, hot);
    }

    /// Paint the glyph scaled to `size` points square, with stroke widths scaled to match.
    /// Ink and opacity follow the same rules as [`Self::paint`].
    pub(crate) fn paint_sized(self, painter: &Painter, center: Pos2, size: f32, t: Tokens, hot: bool) {
        let scale = size / 16.0;
        let at = |x, y| center + vec2(x - 8.0, y - 8.0) * scale;
        let rect = |left, top, right, bottom| Rect::from_min_max(at(left, top), at(right, bottom));
        let ink = if hot { t.text } else { t.text_dim };
        let stroke = Stroke::new(1.3 * scale, ink);
        let accent = Stroke::new(1.4 * scale, t.accent);
        let line = |points: &[(f32, f32)], stroke| {
            painter.add(Shape::line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
        };
        let filled = |pieces: &[&[(f32, f32)]]| {
            let mut mesh = egui::Mesh::default();
            for piece in pieces {
                let first = mesh.vertices.len() as u32;
                for &(x, y) in *piece {
                    mesh.colored_vertex(at(x, y), ink);
                }
                for vertex in 1..piece.len() as u32 - 1 {
                    mesh.add_triangle(first, first + vertex, first + vertex + 1);
                }
            }
            painter.add(Shape::mesh(mesh));
        };
        match self {
            Self::Home => {
                // The roof and the walls either side of the door, so the doorway shows the surface.
                filled(&[
                    &[(8.0, 2.2), (13.6, 6.9), (13.6, 8.9), (2.4, 8.9), (2.4, 6.9)],
                    &[(2.4, 8.9), (6.6, 8.9), (6.6, 13.4), (3.4, 13.4), (2.4, 12.4)],
                    &[(9.4, 8.9), (13.6, 8.9), (13.6, 12.4), (12.6, 13.4), (9.4, 13.4)],
                ]);
            }
            Self::New => {
                line(&[(8.0, 13.5), (3.0, 13.5), (3.0, 2.0), (8.0, 2.0), (11.0, 5.0), (11.0, 7.5)], stroke);
                line(&[(8.0, 2.0), (8.0, 5.0), (11.0, 5.0)], stroke);
                line(&[(9.0, 11.0), (15.0, 11.0)], accent);
                line(&[(12.0, 8.0), (12.0, 14.0)], accent);
            }
            Self::Open => {
                line(&[(2.0, 12.5), (2.0, 3.5), (6.0, 3.5), (7.5, 5.0), (13.0, 5.0), (13.0, 7.0)], stroke);
                line(&[(2.0, 12.5), (4.0, 7.0), (15.0, 7.0), (13.0, 12.5), (2.0, 12.5)], stroke);
            }
            Self::Save => {
                // The pieces surround the two cutouts, so the disk works on any surface.
                let pieces: &[&[(f32, f32)]] = &[
                    &[(4.5, 2.5), (5.5, 2.5), (5.5, 6.0), (2.5, 6.0), (2.5, 4.5)],
                    &[(9.75, 2.5), (10.5, 2.5), (13.5, 5.5), (13.5, 6.0), (9.75, 6.0)],
                    &[(2.5, 6.0), (13.5, 6.0), (13.5, 8.5), (2.5, 8.5)],
                    &[(2.5, 8.5), (5.1, 8.5), (5.1, 12.0), (2.5, 12.0)],
                    &[(10.9, 8.5), (13.5, 8.5), (13.5, 12.0), (10.9, 12.0)],
                    &[(2.5, 12.0), (13.5, 12.0), (12.0, 13.5), (4.0, 13.5)],
                ];
                filled(pieces);
            }
            Self::Download => {
                line(&[(8.0, 2.0), (8.0, 9.5)], accent);
                line(&[(5.0, 6.5), (8.0, 9.5), (11.0, 6.5)], accent);
                line(&[(2.5, 10.5), (2.5, 13.5), (13.5, 13.5), (13.5, 10.5)], stroke);
            }
            Self::Rename => {
                painter.rect_stroke(rect(2.0, 4.5, 14.0, 11.5), 1, stroke, StrokeKind::Middle);
                line(&[(4.5, 8.0), (7.0, 8.0)], stroke);
                line(&[(10.0, 2.5), (10.0, 13.5)], accent);
            }
            Self::Close => {
                line(&[(4.5, 4.5), (11.5, 11.5)], stroke);
                line(&[(11.5, 4.5), (4.5, 11.5)], stroke);
            }
            Self::Undo | Self::Redo => {
                let x = |x| if matches!(self, Self::Redo) { 16.0 - x } else { x };
                let curve = [at(x(3.0), 6.0), at(x(10.0), 5.0), at(x(13.0), 8.0), at(x(13.0), 12.0)];
                painter.add(egui::epaint::CubicBezierShape::from_points_stroke(curve, false, egui::Color32::TRANSPARENT, stroke));
                line(&[(x(6.0), 3.0), (x(3.0), 6.0), (x(6.0), 9.0)], stroke);
            }
            Self::Settings => {
                painter.circle_stroke(center, 4.7 * scale, stroke);
                painter.circle_stroke(center, 1.7 * scale, stroke);
                for step in 0..8 {
                    let direction = egui::Vec2::angled(f32::from(step as u8) * std::f32::consts::FRAC_PI_4);
                    painter.line_segment([center + direction * (4.7 * scale), center + direction * (6.4 * scale)], stroke);
                }
            }
            Self::Bug => {
                line(&[(5.0, 2.5), (7.0, 5.0), (9.0, 5.0), (11.0, 2.5)], stroke);
                for y in [6.0, 9.0, 12.0] {
                    line(&[(2.0, y), (14.0, y)], stroke);
                }
                painter.rect_filled(rect(4.5, 4.5, 11.5, 13.5), 3, ink);
            }
            Self::Appearance => {
                // A disc, half filled: the light and dark palettes.
                painter.circle_stroke(center, 6.0 * scale, stroke);
                filled(&[&[(8.0, 2.0), (10.3, 2.5), (12.2, 3.8), (13.5, 5.7), (14.0, 8.0), (13.5, 10.3), (12.2, 12.2), (10.3, 13.5), (8.0, 14.0)]]);
            }
            Self::Navigation => {
                // Four arrows out from the middle: pan in any direction.
                line(&[(8.0, 2.0), (8.0, 14.0)], stroke);
                line(&[(2.0, 8.0), (14.0, 8.0)], stroke);
                for points in [
                    [(6.0, 4.0), (8.0, 2.0), (10.0, 4.0)],
                    [(6.0, 12.0), (8.0, 14.0), (10.0, 12.0)],
                    [(4.0, 6.0), (2.0, 8.0), (4.0, 10.0)],
                    [(12.0, 6.0), (14.0, 8.0), (12.0, 10.0)],
                ] {
                    line(&points, stroke);
                }
            }
            Self::Keyboard => {
                painter.rect_stroke(rect(1.5, 4.0, 14.5, 12.5), 2, stroke, StrokeKind::Middle);
                for x in [4.5, 7.0, 9.5, 12.0] {
                    painter.circle_filled(at(x, 6.8), 0.6 * scale, ink);
                }
                line(&[(5.0, 9.8), (11.0, 9.8)], stroke);
            }
            Self::Assistant => {
                // A speech bubble with a spark: the AI page.
                painter.rect_stroke(rect(2.0, 3.0, 14.0, 11.5), 3, stroke, StrokeKind::Middle);
                line(&[(5.0, 11.5), (4.5, 14.0), (8.0, 11.5)], stroke);
                line(&[(8.0, 5.0), (8.0, 9.5)], accent);
                line(&[(5.75, 7.25), (10.25, 7.25)], accent);
            }
            Self::Search => {
                painter.circle_stroke(at(7.0, 7.0), 4.3 * scale, stroke);
                line(&[(10.2, 10.2), (14.0, 14.0)], stroke);
            }
            Self::Help => {
                painter.circle_stroke(center, 6.2 * scale, stroke);
                line(&[(6.0, 6.0), (7.0, 5.0), (9.0, 5.0), (10.0, 6.0), (10.0, 7.0), (8.0, 8.5), (8.0, 9.0)], stroke);
                painter.circle_filled(at(8.0, 11.0), 0.7 * scale, ink);
            }
        }
    }
}
