//! Definition previews share the physical board painter without creating a project placement.
use pcbcraft_chrome::{board, properties};
use pcbcraft_engine::board::definition::{CopperBinding, FootprintDefinition, GraphicPurpose, PadShape};

/// Preview navigation is local presentation state and never changes a pinned definition.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Camera {
    zoom: f32,
    pan: egui::Vec2,
}
impl Default for Camera {
    fn default() -> Self {
        Self { zoom: 1., pan: egui::Vec2::ZERO }
    }
}

impl Camera {
    /// Validate command-supplied navigation before publishing it to a pane. Values
    /// are relative to Fit and in logical canvas points, never authored coordinates.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn configured(&self, fit: bool, zoom: Option<f32>, pan: Option<egui::Vec2>) -> Result<Self, String> {
        if fit && (zoom.is_some() || pan.is_some()) {
            return Err("Fit cannot be combined with zoom or pan".into());
        }
        let mut next = if fit { Self::default() } else { self.clone() };
        if let Some(zoom) = zoom {
            if !zoom.is_finite() || !(0.25..=16.0).contains(&zoom) {
                return Err("Preview zoom must be between 25 and 1600 percent".into());
            }
            next.zoom = zoom;
        }
        if let Some(pan) = pan {
            if !pan.is_finite() || pan.abs().max_elem() > 100_000.0 {
                return Err("Preview pan is out of range".into());
            }
            next.pan = pan;
        }
        Ok(next)
    }

    /// Inspect disposable camera state without granting definition edit authority.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inspect(&self) -> serde_json::Value {
        serde_json::json!({"zoom_percent":self.zoom*100.0,"pan":[self.pan.x,self.pan.y]})
    }

    /// Consume wheel zoom inside the canvas before its enclosing inspector scrolls.
    /// Panning remains view-only and uses the same drag gesture as the footprint picker.
    fn navigate(&mut self, ui: &mut egui::Ui, response: &egui::Response, rect: egui::Rect) {
        if response.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta != 0. {
                let zoom = (self.zoom * (delta * 0.005).exp()).clamp(0.25, 16.);
                if let Some(pointer) = response.hover_pos() {
                    let offset = pointer - rect.center();
                    self.pan = offset - (offset - self.pan) * (zoom / self.zoom);
                }
                self.zoom = zoom;
                ui.input_mut(|i| {
                    i.smooth_scroll_delta = egui::Vec2::ZERO;
                });
            }
        }
        if response.dragged() {
            self.pan += ui.input(|i| i.pointer.delta());
        }
    }
}

/// A large read-only preview supports fit, pointer-centred zoom and dragging to pan.
pub(super) fn interactive_footprint(ui: &mut egui::Ui, definition: &FootprintDefinition, camera: &mut Camera) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("2D preview").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if pcbcraft_chrome::controls::button(ui, "Fit").on_hover_text("Fit footprint preview").clicked() {
                *camera = Camera::default();
            }
            let zoom_in = pcbcraft_chrome::controls::button(ui, "+").on_hover_text("Zoom footprint preview in");
            zoom_in.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Zoom footprint preview in"));
            if zoom_in.clicked() {
                camera.zoom = (camera.zoom * 1.25).min(16.);
            }
            let zoom_out = pcbcraft_chrome::controls::button(ui, "−").on_hover_text("Zoom footprint preview out");
            zoom_out.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Zoom footprint preview out"));
            if zoom_out.clicked() {
                camera.zoom = (camera.zoom / 1.25).max(0.25);
            }
        });
    });
    let (rect, response) = properties::footprint_preview(ui, "Footprint preview");
    camera.navigate(ui, &response, rect);
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
            reference: false,
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
                reference: false,
            });
        }
    }
    board::footprint(painter, &pads, &graphics, "", rect.center() + pan, board::copper_color(Some(true)), false, false, false, egui::Vec2::ZERO);
}

/// Library canvases share the picker's navigation while retaining their native palettes.
/// Stable caller scopes isolate symbol and footprint cameras and Fit controls.
pub(super) fn workspace_symbol(ui: &mut egui::Ui, definition: &pcbcraft_engine::schematic::SymbolDefinition, camera: &mut Camera) {
    workspace_geometry(ui, "Symbol preview", false, camera, |painter, rect, camera| {
        crate::schematic_canvas::paint_preview_transformed(
            painter,
            definition,
            rect,
            pcbcraft_chrome::Tokens::get().canvas_ink,
            camera.zoom,
            camera.pan,
        );
    });
}

/// Draw the exact footprint revision without creating or changing a board placement.
pub(super) fn workspace_footprint(ui: &mut egui::Ui, definition: &FootprintDefinition, camera: &mut Camera) {
    workspace_geometry(ui, "Footprint preview", true, camera, |painter, rect, camera| {
        paint_footprint(painter, rect, definition, camera.zoom, camera.pan);
    });
}

/// Shared controls sit outside the canvas so wheel input over metadata still scrolls normally.
fn workspace_geometry(ui: &mut egui::Ui, label: &str, footprint: bool, camera: &mut Camera, paint: impl FnOnce(&egui::Painter, egui::Rect, &Camera)) {
    ui.push_id(label, |ui| {
        ui.horizontal(|ui| {
            for (text, factor, action) in [("−", 0.8, "Zoom out"), ("+", 1.25, "Zoom in"), ("Fit", 0., "Fit")] {
                let response = pcbcraft_chrome::controls::button(ui, text);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), format!("{action} {label}")));
                if response.on_hover_text(format!("{action} {label}")).clicked() {
                    if factor == 0. {
                        *camera = Camera::default();
                    } else {
                        camera.zoom = (camera.zoom * factor).clamp(0.25, 16.);
                    }
                }
            }
        });
        let (rect, response) = properties::geometry_preview(ui, label, footprint);
        if response.double_clicked() {
            *camera = Camera::default();
        }
        camera.navigate(ui, &response, rect);
        response.clone().on_hover_cursor(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        paint(&ui.painter_at(response.rect), rect, camera);
    });
}

/// Part mapping inspection highlights a terminal on its actual unit, including non-default units.
pub(super) fn part_symbol(ui: &mut egui::Ui, definition: &pcbcraft_engine::schematic::SymbolDefinition, terminal: Option<&str>, camera: &mut Camera) {
    workspace_geometry(ui, "Symbol preview", false, camera, |painter, rect, camera| {
        let unit = terminal
            .and_then(|terminal| definition.units().iter().find(|unit| unit.pins().iter().any(|pin| pin.number == terminal)))
            .unwrap_or(definition.default_unit());
        crate::schematic_canvas::paint_part_preview(painter, unit, terminal, rect, camera.zoom, camera.pan);
    });
}
/// The selected table row identifies one physical pad without creating a board assignment.
pub(super) fn part_footprint(ui: &mut egui::Ui, definition: &FootprintDefinition, pad: Option<&str>, camera: &mut Camera) {
    workspace_geometry(ui, "Footprint preview", true, camera, |painter, rect, camera| {
        paint_footprint(painter, rect, definition, camera.zoom, camera.pan);
        if let Some(pad) = pad.and_then(|number| definition.pads.iter().find(|pad| pad.number == number)) {
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
            let scale = rect.width().min(rect.height()) / (2. * extent) * camera.zoom;
            let at = rect.center() + camera.pan + egui::vec2(pad.center.x_nm as f32, pad.center.y_nm as f32) * scale;
            painter.circle_stroke(
                at,
                (pad.width_nm.max(pad.height_nm) as f32 * scale * 0.6).max(5.),
                egui::Stroke::new(2., pcbcraft_chrome::Tokens::get().accent),
            );
        }
    });
}
