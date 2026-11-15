//! Original named-net drawings, with geometry and hit bounds shared by editor presentation.
use crate::{Tokens, theme};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2, vec2};

/// Visual treatment independent of electrical graph semantics.
#[derive(Clone, Copy)]
pub enum Style {
    Local,
    Global,
}

/// Optional net-name appearance uses physical screen offsets supplied by the editor.
#[derive(Clone, Copy)]
pub struct Text {
    pub offset: Option<Vec2>,
    pub height: f32,
    pub rotation: u16,
    pub align: egui::Align,
    pub visible: bool,
    pub show_name: bool,
}

/// Exact name geometry is independent of the electrical anchor and glyph bounds.
pub struct NameInk {
    pub anchor: Pos2,
    pub bounds: Rect,
}

struct Ink {
    bounds: Rect,
    name: Option<NameInk>,
}

/// Paint an upright name and quarter-turned anchor glyph. Return the complete ink/anchor
/// bounds so pointer hit testing follows the actual text at every interface scale and zoom.
/// Connected anchors omit their open-pin ring; callers resolve actual local contact.
#[allow(clippy::too_many_arguments)]
pub fn paint(painter: &Painter, at: Pos2, name: &str, style: Style, rotation: u16, step: f32, color: Color32, connected: bool) -> Rect {
    draw(painter, at, name, style, rotation, step, color, true, connected, None).bounds
}

/// Measure the same glyph and text without adding shapes to the scene.
pub fn bounds(painter: &Painter, at: Pos2, name: &str, style: Style, rotation: u16, step: f32) -> Rect {
    draw(painter, at, name, style, rotation, step, Tokens::get().text, false, false, None).bounds
}

/// Paint an anchor and its independently configured name without changing contact geometry.
#[allow(clippy::too_many_arguments)]
pub fn paint_with_text(
    painter: &Painter,
    at: Pos2,
    name: &str,
    style: Style,
    rotation: u16,
    step: f32,
    color: Color32,
    connected: bool,
    text: Option<&Text>,
) -> Rect {
    draw(painter, at, name, style, rotation, step, color, true, connected, text).bounds
}

/// Measure exactly the text/glyph ink painted by `paint_with_text` for picking and selection.
#[allow(clippy::too_many_arguments)]
pub fn bounds_with_text(painter: &Painter, at: Pos2, name: &str, style: Style, rotation: u16, step: f32, text: Option<&Text>) -> Rect {
    draw(painter, at, name, style, rotation, step, Tokens::get().text, false, false, text).bounds
}

/// Measure only the visible net name using the exact painter layout, including manual offsets,
/// alignment and rotation. Hidden names return None and cannot acquire independent text input.
#[allow(clippy::too_many_arguments)]
pub fn name_ink(painter: &Painter, at: Pos2, name: &str, style: Style, rotation: u16, step: f32, text: Option<&Text>) -> Option<NameInk> {
    draw(painter, at, name, style, rotation, step, Tokens::get().text, false, false, text).name
}

/// Keep text measurement and painted geometry aligned for hit testing.
#[allow(clippy::too_many_arguments)]
fn draw(
    painter: &Painter,
    at: Pos2,
    name: &str,
    style: Style,
    rotation: u16,
    step: f32,
    color: Color32,
    paint: bool,
    connected: bool,
    text: Option<&Text>,
) -> Ink {
    let mut name_ink = None;
    let scale = (step / 10.0).max(0.4);
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
            let width =
                crate::canvas_text::layout_no_wrap(painter, name.into(), FontId::proportional(Tokens::get().font_label * scale), color).size().x
                    / scale
                    + 14.;
            let points = [vec2(0., 0.), vec2(7., -9.), vec2(width, -9.), vec2(width, 9.), vec2(7., 9.), vec2(0., 0.)];
            for pair in points.windows(2) {
                line(pair[0], pair[1]);
            }
            vec2(width / 2. + 3., 0.)
        }
    };
    if text.is_none_or(|text| text.visible) {
        let label_at = text.and_then(|t| t.offset).map_or_else(|| transform(offset), |offset| at + offset);
        let align = text.map_or_else(
            || {
                if matches!(style, Style::Local) {
                    if rotation == 180 { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }
                } else {
                    Align2::CENTER_CENTER
                }
            },
            |t| Align2([t.align, egui::Align::Center]),
        );
        let size = text.map_or(step, |t| t.height);
        let name = if text.is_some_and(|t| t.show_name) { format!("Net name: {name}") } else { name.to_owned() };
        let galley = crate::canvas_text::layout_no_wrap(painter, name, FontId::proportional(size.max(1.)), color);
        let text_bounds = align.anchor_size(label_at, galley.size());
        let mut shape = egui::epaint::TextShape::new(text_bounds.min, galley, color);
        if let Some(text) = text {
            shape = shape.with_angle_and_anchor(-(text.rotation as f32).to_radians(), align);
        } else if matches!(style, Style::Global) {
            shape = shape.with_angle_and_anchor(-(rotation as f32).to_radians(), Align2::CENTER_CENTER);
        }
        let name_bounds = shape.visual_bounding_rect();
        bounds = bounds.union(name_bounds);
        name_ink = Some(NameInk { anchor: label_at, bounds: name_bounds });
        if paint {
            painter.add(shape);
        }
    }
    // A small anchor marks the exact electrical contact, independent of glyph orientation.
    if paint && !connected {
        painter.circle_stroke(at, 2.0, stroke);
    }
    Ink { bounds: bounds.expand(3.), name: name_ink }
}
