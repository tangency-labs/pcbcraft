//! Original named-net drawings, with geometry and hit bounds shared by editor presentation.
use crate::{Tokens, theme};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2, vec2};

/// Visual treatment independent of electrical graph semantics.
#[derive(Clone, Copy)]
pub enum Style {
    Local,
    Global,
    Power,
    /// Flat-bar supply glyph sharing the power arrow's anchor and upright net-name placement.
    PowerBar,
    Ground,
}

/// Paint an upright name and quarter-turned anchor glyph. Return the complete ink/anchor
/// bounds so pointer hit testing follows the actual text at every interface scale and zoom.
/// Connected anchors omit their open-pin ring; callers resolve actual local contact.
#[allow(clippy::too_many_arguments)]
pub fn paint(painter: &Painter, at: Pos2, name: &str, style: Style, rotation: u16, step: f32, color: Color32, connected: bool) -> Rect {
    draw(painter, at, name, style, rotation, step, color, true, connected)
}

/// Measure the same glyph and text without adding shapes to the scene.
pub fn bounds(painter: &Painter, at: Pos2, name: &str, style: Style, rotation: u16, step: f32) -> Rect {
    draw(painter, at, name, style, rotation, step, Tokens::get().text, false, false)
}

/// Keep text measurement and painted geometry aligned for hit testing.
#[allow(clippy::too_many_arguments)]
fn draw(painter: &Painter, at: Pos2, name: &str, style: Style, rotation: u16, step: f32, color: Color32, paint: bool, connected: bool) -> Rect {
    let scale = (step / 10.0).clamp(0.4, 3.0);
    let transform = |v: Vec2| {
        let v = match rotation {
            90 => vec2(v.y, -v.x),
            180 => -v,
            270 => vec2(-v.y, v.x),
            _ => v,
        };
        at + v * scale
    };
    let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color);
    let mut bounds = Rect::from_center_size(at, vec2(8., 8.));
    let mut line = |a, b| {
        let a = transform(a);
        let b = transform(b);
        if paint {
            painter.line_segment([a, b], stroke);
        }
        bounds = bounds.union(Rect::from_two_pos(a, b));
    };
    let offset = match style {
        Style::Local => vec2(4., -10.),
        Style::Global => {
            let width = painter.layout_no_wrap(name.into(), FontId::proportional(Tokens::get().font_label * scale), color).size().x / scale + 14.;
            let points = [vec2(0., 0.), vec2(7., -9.), vec2(width, -9.), vec2(width, 9.), vec2(7., 9.), vec2(0., 0.)];
            for pair in points.windows(2) {
                line(pair[0], pair[1]);
            }
            vec2(width / 2. + 3., 0.)
        }
        Style::Power => {
            line(Vec2::ZERO, vec2(0., -20.));
            line(vec2(-7., -13.), vec2(0., -20.));
            line(vec2(0., -20.), vec2(7., -13.));
            vec2(0., -30.)
        }
        Style::PowerBar => {
            line(Vec2::ZERO, vec2(0., -20.));
            line(vec2(-10., -20.), vec2(10., -20.));
            vec2(0., -30.)
        }
        Style::Ground => {
            line(Vec2::ZERO, vec2(0., 12.));
            for (y, width) in [(12., 10.), (17., 6.), (22., 2.)] {
                line(vec2(-width, y), vec2(width, y));
            }
            vec2(0., 32.)
        }
    };
    let label_at = transform(offset);
    let align =
        if matches!(style, Style::Local) { if rotation == 180 { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER } } else { Align2::CENTER_CENTER };
    let galley = painter.layout_no_wrap(name.into(), FontId::proportional(Tokens::get().font_label * scale), color);
    let text_bounds = align.anchor_size(label_at, galley.size());
    let mut text = egui::epaint::TextShape::new(text_bounds.min, galley, color);
    if matches!(style, Style::Global) {
        text = text.with_angle_and_anchor(-(f32::from(rotation)).to_radians(), Align2::CENTER_CENTER);
    }
    bounds = bounds.union(text.visual_bounding_rect());
    if paint {
        painter.add(text);
    }
    // A small anchor marks the exact electrical contact, independent of glyph orientation.
    if paint && !connected {
        painter.circle_stroke(at, 2.0, stroke);
    }
    bounds.expand(3.)
}
