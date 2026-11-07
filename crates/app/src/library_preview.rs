//! Definition previews share the physical board painter without creating a project placement.
use pcbcraft_chrome::{board, properties};
use pcbcraft_engine::board::definition::{CopperBinding, FootprintDefinition, GraphicPurpose, PadShape};

/// Project native local coordinates into a bounded preview, including rotated pad dimensions
/// and technical graphics. Copper remains distinct from fabrication/courtyard drawing.
pub(super) fn footprint(ui: &mut egui::Ui, definition: &FootprintDefinition) {
    properties::preview(ui, |painter, rect| {
        let extent = definition
            .pads
            .iter()
            .map(|p| (p.center, p.width_nm, p.height_nm))
            .chain(definition.graphics.iter().map(|g| (g.center, g.width_nm, g.height_nm)))
            .fold(1_000_000.0_f32, |extent, (p, w, h)| extent.max(p.x_nm.abs().max(p.y_nm.abs()) as f32 + w.max(h) as f32));
        let scale = rect.width().min(rect.height()) / (2.0 * extent);
        let at = |point: pcbcraft_engine::schematic::Point| rect.center() + egui::vec2(point.x_nm as f32, point.y_nm as f32) * scale;
        let corners = |center: pcbcraft_engine::schematic::Point, width: i64, height: i64, rotation: u16| {
            let angle = (rotation as f32).to_radians();
            [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(x, y)| {
                let (x, y) = (x * width as f32 * scale, y * height as f32 * scale);
                at(center) + egui::vec2(x * angle.cos() - y * angle.sin(), x * angle.sin() + y * angle.cos())
            })
        };
        let pads: Vec<_> = definition
            .pads
            .iter()
            .map(|pad| board::PadDrawing {
                number: pad.number.clone(),
                center: at(pad.center),
                copper: Some(corners(pad.center, pad.width_nm, pad.height_nm, pad.rotation)),
                color: Some(if pad.copper == CopperBinding::AllCopper {
                    pcbcraft_chrome::Tokens::get().board_through
                } else {
                    board::copper_color(Some(true))
                }),
                contour: match pad.shape {
                    PadShape::Rectangle => board::PadContour::Rectangle,
                    PadShape::Circle => board::PadContour::Circle,
                    PadShape::RoundedRectangle => board::PadContour::RoundedRectangle { radius: pad.corner_radius_nm as f32 * scale },
                },
                drill_radius: pad.drill_nm.map(|d| d as f32 * scale / 2.0),
                through_hole: pad.copper == CopperBinding::AllCopper,
                net: None,
            })
            .collect();
        let graphics: Vec<_> = definition
            .graphics
            .iter()
            .map(|g| board::GraphicDrawing {
                points: corners(g.center, g.width_nm, g.height_nm, g.rotation).to_vec(),
                style: match g.purpose {
                    GraphicPurpose::Fabrication => board::GraphicStyle::Fabrication,
                    GraphicPurpose::Silkscreen => board::GraphicStyle::Silkscreen,
                    GraphicPurpose::Courtyard => board::GraphicStyle::Courtyard,
                },
                color: None,
                closed: true,
                stroke_width: None,
            })
            .collect();
        board::footprint(painter, &pads, &graphics, "", rect.center(), board::copper_color(Some(true)), false, false);
    });
}
