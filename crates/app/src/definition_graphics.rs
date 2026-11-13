//! Project parameterized native artwork through the shared palette and physical coordinates.
use egui::{Color32, Painter, Pos2, Stroke};
use pcbcraft_engine::{
    library_graphics::{Fill, Graphic},
    schematic::Point,
};

/// Paint editable geometry with its physical line width; chrome supplies colors rather than
/// storing theme-dependent ink in the definition. Open paths and text remain unfilled.
pub(super) fn paint(p: &Painter, graphic: &Graphic, at: impl Fn(Point) -> Pos2, scale: f32, ink: Color32, background: Color32) {
    let stroke = Stroke::new((graphic.stroke_nm as f32 * scale).max(pcbcraft_chrome::authoring::MIN_DRAWING_STROKE), ink);
    for path in graphic.paths() {
        let mut points: Vec<_> = path.into_iter().map(&at).collect();
        if graphic.closed() && graphic.fill != Fill::None && points.len() >= 3 {
            p.add(egui::Shape::convex_polygon(points.clone(), if graphic.fill == Fill::Solid { ink } else { background }, Stroke::NONE));
        }
        if graphic.closed()
            && points.first() != points.last()
            && let Some(first) = points.first().copied()
        {
            points.push(first);
        }
        if points.len() >= 2 {
            p.add(egui::Shape::line(points, stroke));
        }
    }
}

/// Keep native lettering readable after schematic placement mirroring and rotation, retaining
/// its interior alignment when a half turn would otherwise put labels upside down. No file
/// geometry changes.
pub(super) fn symbol_text(graphic: &Graphic, turns: u8, mirrored: bool) -> Graphic {
    use pcbcraft_engine::{
        library_graphics::Geometry,
        schematic::{TextAlign, rotate},
    };
    let mut copy = graphic.clone();
    if let Geometry::Text { at, rotation, align, .. } = &mut copy.geometry {
        if mirrored {
            at.x_nm = -at.x_nm;
            *rotation = (180 - i32::from(*rotation)).rem_euclid(360) as u16;
        }
        (at.x_nm, at.y_nm) = rotate(at.x_nm, at.y_nm, turns);
        let mut degrees = (i32::from(*rotation) - 90 * i32::from(turns % 4)).rem_euclid(360);
        if degrees > 90 && degrees <= 270 {
            degrees = (degrees + 180) % 360;
            *align = match align {
                TextAlign::Start => TextAlign::End,
                TextAlign::End => TextAlign::Start,
                TextAlign::Middle => TextAlign::Middle,
            };
        }
        *rotation = degrees as u16;
    }
    copy
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::{
        library_graphics::{Fill, Geometry},
        schematic::TextAlign,
    };
    /// A half-turn label stays readable and inside its rotated body edge without mutating native data.
    #[test]
    fn rotated_symbol_text_keeps_its_alignment_and_source() {
        let original = Graphic {
            geometry: Geometry::Text {
                at: Point { x_nm: 2_000_000, y_nm: 0 },
                text: "IN".into(),
                height_nm: 1_000_000,
                rotation: 0,
                align: TextAlign::Start,
            },
            stroke_nm: 100_000,
            fill: Fill::None,
        };
        let rotated = symbol_text(&original, 2, false);
        assert!(matches!(rotated.geometry, Geometry::Text { at: Point { x_nm: -2_000_000, y_nm: 0 }, rotation: 0, align: TextAlign::End, .. }));
        assert!(matches!(original.geometry, Geometry::Text { rotation: 0, align: TextAlign::Start, .. }));
        assert_eq!(symbol_text(&original, 4, false), original);
    }
}
