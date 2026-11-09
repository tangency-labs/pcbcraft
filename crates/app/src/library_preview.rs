//! Definition previews share the physical board painter without creating a project placement.
use pcbcraft_chrome::{board, properties};
use pcbcraft_engine::board::definition::{CopperBinding, FootprintDefinition, GraphicPurpose, PadShape};

/// Preview navigation is local presentation state and never changes a pinned definition.
#[derive(Debug)]
pub(super) struct FootprintCamera {
    zoom: f32,
    pan: egui::Vec2,
}
impl Default for FootprintCamera {
    fn default() -> Self {
        Self { zoom: 1., pan: egui::Vec2::ZERO }
    }
}

/// A large read-only preview supports fit, pointer-centred zoom and dragging to pan.
pub(super) fn interactive_footprint(ui: &mut egui::Ui, definition: &FootprintDefinition, camera: &mut FootprintCamera) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("2D preview").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Fit").on_hover_text("Fit footprint preview").clicked() {
                *camera = FootprintCamera::default();
            }
            let zoom_in = ui.button("+").on_hover_text("Zoom footprint preview in");
            zoom_in.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Zoom footprint preview in"));
            if zoom_in.clicked() {
                camera.zoom = (camera.zoom * 1.25).min(16.);
            }
            let zoom_out = ui.button("−").on_hover_text("Zoom footprint preview out");
            zoom_out.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Zoom footprint preview out"));
            if zoom_out.clicked() {
                camera.zoom = (camera.zoom / 1.25).max(0.25);
            }
        });
    });
    let (rect, response) = properties::footprint_preview(ui, "Footprint preview");
    if response.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        if delta != 0. {
            let zoom = (camera.zoom * (delta * 0.005).exp()).clamp(0.25, 16.);
            if let Some(pointer) = response.hover_pos() {
                let offset = pointer - rect.center();
                camera.pan = offset - (offset - camera.pan) * (zoom / camera.zoom);
            }
            camera.zoom = zoom;
            ui.input_mut(|i| {
                i.smooth_scroll_delta = egui::Vec2::ZERO;
            });
        }
    }
    if response.dragged() {
        camera.pan += ui.input(|i| i.pointer.delta());
    }
    paint_footprint(&ui.painter_at(response.rect), rect, definition, camera.zoom, camera.pan);
}

/// Draw the pinned native pad and technical geometry without creating a board placement.
fn paint_footprint(painter: &egui::Painter, rect: egui::Rect, definition: &FootprintDefinition, zoom: f32, pan: egui::Vec2) {
    let mut extent = definition
        .pads
        .iter()
        .map(|p| (p.center, p.width_nm, p.height_nm))
        .chain(definition.graphics.iter().map(|g| (g.center, g.width_nm, g.height_nm)))
        .fold(1_000_000.0_f32, |extent, (p, w, h)| extent.max(p.x_nm.abs().max(p.y_nm.abs()) as f32 + w.max(h) as f32));
    for artwork in &definition.artwork {
        for p in artwork.graphic.paths().iter().flatten() {
            extent = extent.max(p.x_nm.abs().max(p.y_nm.abs()) as f32 + artwork.graphic.stroke_nm as f32);
        }
    }
    let scale = rect.width().min(rect.height()) / (2.0 * extent) * zoom;
    let at = |point: pcbcraft_engine::schematic::Point| rect.center() + pan + egui::vec2(point.x_nm as f32, point.y_nm as f32) * scale;
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
                PadShape::Oval => board::PadContour::Oval,
                PadShape::RoundedRectangle => board::PadContour::RoundedRectangle { radius: pad.corner_radius_nm as f32 * scale },
            },
            drill_radius: pad.drill_nm.map(|d| d as f32 * scale / 2.0),
            through_hole: pad.copper == CopperBinding::AllCopper,
            net: None,
        })
        .collect();
    let mut graphics: Vec<_> = definition
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
    for artwork in &definition.artwork {
        for points in artwork.graphic.paths() {
            graphics.push(board::GraphicDrawing {
                color: None,
                points: points.into_iter().map(at).collect(),
                style: match artwork.purpose {
                    GraphicPurpose::Fabrication => board::GraphicStyle::Fabrication,
                    GraphicPurpose::Silkscreen => board::GraphicStyle::Silkscreen,
                    GraphicPurpose::Courtyard => board::GraphicStyle::Courtyard,
                },
                closed: artwork.graphic.closed(),
                stroke_width: Some(artwork.graphic.stroke_nm as f32 * scale),
            });
        }
    }
    board::footprint(painter, &pads, &graphics, "", rect.center() + pan, board::copper_color(Some(true)), false, false);
}
