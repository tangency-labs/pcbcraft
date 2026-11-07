//! Original canvas tools project native primitives through existing Craft painters.
use super::*;
use egui::{Rect, vec2};
use pcbcraft_chrome::{Tokens, board, grid, workspace};
use pcbcraft_engine::{
    authoring::Primitive,
    board::definition::{self as foot, CopperBinding, GraphicPurpose, PadShape},
    schematic::{ElectricalType, Point, TextAlign},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Place {
    Pin,
    Pad,
    Line,
    Rectangle,
    Circle,
    Text,
    Outline,
    Silkscreen,
}
#[derive(Clone, Debug, Default)]
pub(super) enum Tool {
    #[default]
    Select,
    Draw {
        kind: Place,
        start: Option<Point>,
        revision: u64,
    },
    Move {
        object: u64,
        original: Primitive,
        revision: u64,
    },
}
/// Human-readable browser labels keep electrical numbers visible beside stable primitive IDs.
pub(super) fn name(primitive: &Primitive) -> String {
    match primitive {
        Primitive::Pin { number, .. } => format!("Pin {number}"),
        Primitive::Pad { pad } => format!("Pad {}", pad.number),
        Primitive::Line { .. } => "Line".into(),
        Primitive::Rectangle { .. } => "Rectangle".into(),
        Primitive::Circle { .. } => "Circle".into(),
        Primitive::Text { .. } => "Text".into(),
        Primitive::Polyline { .. } => "Polyline".into(),
        Primitive::Outline { graphic } => format!("{:?} rectangle", graphic.purpose),
        Primitive::Silkscreen { .. } => "Silkscreen".into(),
    }
}
/// Extents include copper/technical dimensions rather than only their centers. Circumscribed
/// bounds keep rotated lands visible during Fit; hit testing uses the same projected bounds.
fn points(primitive: &Primitive) -> Vec<Point> {
    let rectangle = |center: Point, w: i64, h: i64| {
        let radius = ((w as f64).hypot(h as f64) / 2.0).ceil() as i64;
        vec![Point { x_nm: center.x_nm - radius, y_nm: center.y_nm - radius }, Point { x_nm: center.x_nm + radius, y_nm: center.y_nm + radius }]
    };
    match primitive {
        Primitive::Pin { at, .. } | Primitive::Text { at, .. } => vec![*at],
        Primitive::Line { from, to } => vec![*from, *to],
        Primitive::Rectangle { min, max } => vec![*min, *max],
        Primitive::Circle { center, radius_nm } => vec![
            Point { x_nm: center.x_nm - radius_nm, y_nm: center.y_nm - radius_nm },
            Point { x_nm: center.x_nm + radius_nm, y_nm: center.y_nm + radius_nm },
        ],
        Primitive::Polyline { points } => points.clone(),
        Primitive::Pad { pad } => rectangle(pad.center, pad.width_nm, pad.height_nm),
        Primitive::Outline { graphic } => rectangle(graphic.center, graphic.width_nm, graphic.height_nm),
        Primitive::Silkscreen { path } => path.points.clone(),
    }
}
impl Document {
    /// Tools do not mutate definitions until a completed click gesture passes revision validation.
    pub(super) fn tools(&mut self, ui: &mut Ui) {
        let item = self.session.snapshot();
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            authoring::heading(ui, if item.kind == Kind::Symbol { "Symbol Editor" } else { "Footprint Editor" });
            if ui.selectable_label(matches!(self.tool, Tool::Select), "Select").clicked() {
                picked = Some(None);
            }
            let tools = if item.kind == Kind::Symbol {
                vec![(Place::Pin, "Pin"), (Place::Line, "Line"), (Place::Rectangle, "Rectangle"), (Place::Circle, "Circle"), (Place::Text, "Text")]
            } else {
                vec![(Place::Pad, "Pad"), (Place::Outline, "Rectangle"), (Place::Silkscreen, "Silkscreen")]
            };
            for (kind, label) in tools {
                if ui.selectable_label(matches!(self.tool,Tool::Draw{kind:current,..}if current==kind), label).clicked() {
                    picked = Some(Some(kind));
                }
            }
            if ui.button("Move").clicked() {
                self.begin_move();
            }
            if ui.button("Delete").clicked() {
                self.delete();
            }
            if ui.button("Rotate").clicked() {
                self.rotate();
            }
            if ui.button("Fit").clicked() {
                self.camera.request_fit();
            }
            egui::ComboBox::from_id_salt("library_grid").selected_text(format!("Grid {} mm", self.camera.snap_nm() as f64 / 1_000_000.0)).show_ui(
                ui,
                |ui| {
                    let spacings =
                        if item.kind == Kind::Symbol { vec![1_270_000] } else { vec![25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000] };
                    for spacing in spacings {
                        if ui.selectable_label(self.camera.snap_nm() == spacing, format!("{} mm", spacing as f64 / 1_000_000.0)).clicked() {
                            self.camera.set_snap_nm(spacing);
                        }
                    }
                },
            );
        });
        if let Some(kind) = picked {
            if self.pending() {
                self.error = Some("Apply or reset the property fields before changing tools.".into());
            } else {
                self.tool = kind.map_or(Tool::Select, |kind| Tool::Draw { kind, start: None, revision: self.session.revision() });
            }
        }
    }
    /// Move preserves electrical numbering and captures the revision before the preview begins.
    fn begin_move(&mut self) {
        if self.pending() {
            self.error = Some("Apply or reset the property fields before moving.".into());
            return;
        }
        if let Some(object) = self.selected
            && let Some(original) = self.session.snapshot().objects.get(&object)
        {
            self.tool = Tool::Move { object, original: original.clone(), revision: self.session.revision() };
        }
    }
    /// Deletion and invalid last-pin/last-pad removal use the same atomic native validation.
    fn delete(&mut self) {
        if let Some(object) = self.selected {
            let result = self.execute(self.session.revision(), Action::Delete { objects: vec![object] });
            if let Err(error) = result {
                self.error = Some(error);
            }
        }
    }
    /// Physical rectangular primitives support quarter turns; other primitive edits use their
    /// explicit coordinates rather than pretending the native schema stores a pin direction.
    fn rotate(&mut self) {
        let Some(object) = self.selected else { return };
        let item = self.session.snapshot();
        let Some(mut primitive) = item.objects.get(&object).cloned() else { return };
        match &mut primitive {
            Primitive::Pad { pad } => pad.rotation = (pad.rotation + 90) % 360,
            Primitive::Outline { graphic } => graphic.rotation = (graphic.rotation + 90) % 360,
            _ => {
                self.error = Some("Rotation is available for pads and technical rectangles; edit other primitives by their coordinates.".into());
                return;
            }
        }
        if let Err(error) = self.execute(self.session.revision(), Action::Replace { object, primitive }) {
            self.error = Some(error);
        }
    }
    /// Navigate, paint and hit-test the active definition without consulting a project snapshot.
    /// Floating cards exclude canvas input and Fit geometry; Escape cancels previews only.
    pub(super) fn canvas(&mut self, ui: &mut Ui, area: Rect, covered: [Rect; 2], navigation: Navigation) {
        let item = self.session.snapshot();
        let t = Tokens::get();
        let response = ui.interact(area, ui.id().with("library_canvas"), egui::Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), "Library canvas"));
        if !ui.input(|input| input.focused) {
            self.tool = Tool::Select;
        }
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let over = ui.is_enabled()
            && !egui::Popup::is_any_open(ui.ctx())
            && pointer.is_some_and(|point| {
                area.contains(point)
                    && !covered.iter().any(|rect| rect.contains(point))
                    && ui.ctx().layer_id_at(point).is_none_or(|layer| layer.order == egui::Order::Background)
            });
        self.camera.set_uncovered(area, workspace::catalog::uncovered(area, &covered));
        self.camera.navigate_definition(&response, area, item.objects.values().flat_map(points), over, navigation);
        let layer = egui::LayerId::background();
        let painter = ui.ctx().layer_painter(layer).with_clip_rect(area);
        painter.rect_filled(area, 0, if item.kind == Kind::Symbol { t.canvas } else { t.board_canvas });
        let origin = self.camera.screen(Point { x_nm: 0, y_nm: 0 }, area);
        grid::paint(&painter, area, origin, self.camera.snap_step(), authoring::GRID_OPACITY);
        let mut preview = None;
        if let Some(point) = pointer.filter(|_| over) {
            let at = self.camera.snap(point, area);
            preview = match &self.tool {
                Tool::Move { original, .. } => {
                    original.translated(Point { x_nm: at.x_nm - original.anchor().x_nm, y_nm: at.y_nm - original.anchor().y_nm }).ok()
                }
                Tool::Draw { kind, start, .. } => primitive(&item, *kind, start.unwrap_or(at), at).ok(),
                _ => None,
            };
            if response.clicked() {
                match self.tool.clone() {
                    Tool::Select => {
                        let found = item.objects.iter().rev().find(|(_, primitive)| hit(primitive, &self.camera, area, point)).map(|(id, _)| *id);
                        let _ = self.select(found);
                    }
                    Tool::Move { object, revision, .. } => {
                        if let Some(primitive) = preview.clone()
                            && let Err(error) = self.execute(revision, Action::Replace { object, primitive })
                        {
                            self.error = Some(error);
                            self.tool = Tool::Select;
                        }
                    }
                    Tool::Draw { kind, start: None, revision }
                        if matches!(kind, Place::Line | Place::Rectangle | Place::Circle | Place::Outline | Place::Silkscreen) =>
                    {
                        self.tool = Tool::Draw { kind, start: Some(at), revision }
                    }
                    Tool::Draw { kind, revision, .. } => {
                        match primitive(
                            &item,
                            kind,
                            match &self.tool {
                                Tool::Draw { start: Some(start), .. } => *start,
                                _ => at,
                            },
                            at,
                        )
                        .and_then(|primitive| self.execute(revision, Action::Insert { primitive }))
                        {
                            Ok(()) => self.tool = Tool::Draw { kind, start: None, revision: self.session.revision() },
                            Err(error) => {
                                self.error = Some(error);
                                self.tool = Tool::Select;
                            }
                        }
                    }
                }
            }
        }
        if ui.is_enabled() && !ui.ctx().egui_wants_keyboard_input() {
            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                self.tool = Tool::Select;
            }
            if over && ui.input(|input| input.key_pressed(egui::Key::M)) {
                self.begin_move();
            }
            if over && ui.input(|input| input.key_pressed(egui::Key::R)) {
                self.rotate();
            }
            if over && ui.input(|input| input.key_pressed(egui::Key::Delete)) {
                self.delete();
            }
        }
        for (id, primitive) in &item.objects {
            if matches!(self.tool,Tool::Move{object,..}if object==*id) && preview.is_some() {
                continue;
            }
            paint(&painter, primitive, &self.camera, area, self.selected == Some(*id));
        }
        if let Some(primitive) = preview {
            paint(&painter, &primitive, &self.camera, area, true);
        }
        authoring::origin(&painter, origin);
    }
}
/// Allocate the next unused electrical number; object allocation remains inside the engine.
fn primitive(item: &Item, kind: Place, start: Point, end: Point) -> Result<Primitive, String> {
    let number = (1..=513)
        .map(|n| n.to_string())
        .find(|number| {
            !item.objects.values().any(|p| match p {
                Primitive::Pin { number: n, .. } => n == number,
                Primitive::Pad { pad } => &pad.number == number,
                _ => false,
            })
        })
        .ok_or("Electrical number limit reached.")?;
    let min = Point { x_nm: start.x_nm.min(end.x_nm), y_nm: start.y_nm.min(end.y_nm) };
    let max = Point { x_nm: start.x_nm.max(end.x_nm), y_nm: start.y_nm.max(end.y_nm) };
    Ok(match kind {
        Place::Pin => Primitive::Pin { number, name: None, electrical_type: ElectricalType::Passive, at: end },
        Place::Pad => Primitive::Pad {
            pad: foot::RectPad {
                number,
                center: end,
                width_nm: 600_000,
                height_nm: 800_000,
                rotation: 0,
                shape: PadShape::Rectangle,
                copper: CopperBinding::ComponentSide,
                corner_radius_nm: 0,
                drill_nm: None,
            },
        },
        Place::Line => Primitive::Line { from: start, to: end },
        Place::Rectangle => Primitive::Rectangle { min, max },
        Place::Circle => {
            Primitive::Circle { center: start, radius_nm: ((end.x_nm - start.x_nm) as f64).hypot((end.y_nm - start.y_nm) as f64).round() as i64 }
        }
        Place::Text => Primitive::Text { at: end, text: "Text".into(), align: TextAlign::Start },
        Place::Outline => Primitive::Outline {
            graphic: foot::GraphicRect {
                purpose: GraphicPurpose::Fabrication,
                center: Point { x_nm: (min.x_nm + max.x_nm) / 2, y_nm: (min.y_nm + max.y_nm) / 2 },
                width_nm: max.x_nm - min.x_nm,
                height_nm: max.y_nm - min.y_nm,
                rotation: 0,
            },
        },
        Place::Silkscreen => Primitive::Silkscreen { path: foot::SilkPath { points: vec![start, end], closed: false } },
    })
}
/// Project primitive bounds through the active definition camera.
fn bounds(primitive: &Primitive, camera: &crate::canvas_camera::Camera, area: Rect) -> Rect {
    points(primitive).into_iter().fold(Rect::NOTHING, |rect, point| rect.union(Rect::from_pos(camera.screen(point, area))))
}
/// Render symbol ink, numbered copper/drills and technical layers with existing chrome colors.
fn paint(painter: &egui::Painter, primitive: &Primitive, camera: &crate::canvas_camera::Camera, area: Rect, selected: bool) {
    let t = Tokens::get();
    let at = |point| camera.screen(point, area);
    let scale = camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32;
    let stroke = authoring::ink_stroke();
    let corners = |center: Point, w: i64, h: i64, rotation: u16| {
        let angle = (rotation as f32).to_radians();
        [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(x, y)| {
            let (x, y) = (x * w as f32 * scale, y * h as f32 * scale);
            at(center) + vec2(x * angle.cos() - y * angle.sin(), x * angle.sin() + y * angle.cos())
        })
    };
    match primitive {
        Primitive::Pin { at: point, number, name, .. } => {
            let point = at(*point);
            painter.circle_stroke(point, authoring::PIN_RADIUS, stroke);
            painter.text(
                point + authoring::pin_label_offset(),
                egui::Align2::LEFT_BOTTOM,
                format!("{number} {}", name.as_deref().unwrap_or("")),
                egui::FontId::proportional(t.font_small),
                t.canvas_ink,
            );
        }
        Primitive::Line { from, to } => {
            painter.line_segment([at(*from), at(*to)], stroke);
        }
        Primitive::Rectangle { min, max } => {
            painter.rect_stroke(Rect::from_two_pos(at(*min), at(*max)), 0, stroke, egui::StrokeKind::Middle);
        }
        Primitive::Circle { center, radius_nm } => {
            painter.circle_stroke(at(*center), *radius_nm as f32 * scale, stroke);
        }
        Primitive::Text { at: point, text, align } => {
            painter.text(
                at(*point),
                match align {
                    TextAlign::Start => egui::Align2::LEFT_CENTER,
                    TextAlign::Middle => egui::Align2::CENTER_CENTER,
                    TextAlign::End => egui::Align2::RIGHT_CENTER,
                },
                text,
                egui::FontId::proportional(t.font_body),
                t.canvas_ink,
            );
        }
        Primitive::Polyline { points } => {
            painter.add(egui::Shape::line(points.iter().map(|p| at(*p)).collect(), stroke));
        }
        Primitive::Pad { pad } => board::footprint(
            painter,
            &[board::PadDrawing {
                number: pad.number.clone(),
                center: at(pad.center),
                copper: Some(corners(pad.center, pad.width_nm, pad.height_nm, pad.rotation)),
                contour: match pad.shape {
                    PadShape::Rectangle => board::PadContour::Rectangle,
                    PadShape::Circle => board::PadContour::Circle,
                    PadShape::RoundedRectangle => board::PadContour::RoundedRectangle { radius: pad.corner_radius_nm as f32 * scale },
                },
                drill_radius: pad.drill_nm.map(|r| r as f32 * scale / 2.0),
                through_hole: pad.copper == CopperBinding::AllCopper,
                net: None,
            }],
            &[],
            "",
            at(pad.center),
            board::copper_color(Some(true)),
            selected,
            false,
        ),
        Primitive::Outline { graphic } => board::footprint(
            painter,
            &[],
            &[board::GraphicDrawing {
                points: corners(graphic.center, graphic.width_nm, graphic.height_nm, graphic.rotation).to_vec(),
                style: match graphic.purpose {
                    GraphicPurpose::Fabrication => board::GraphicStyle::Fabrication,
                    GraphicPurpose::Silkscreen => board::GraphicStyle::Silkscreen,
                    GraphicPurpose::Courtyard => board::GraphicStyle::Courtyard,
                },
                closed: true,
                stroke_width: None,
            }],
            "",
            at(graphic.center),
            board::copper_color(Some(true)),
            selected,
            false,
        ),
        Primitive::Silkscreen { path } => board::footprint(
            painter,
            &[],
            &[board::GraphicDrawing {
                points: path.points.iter().map(|p| at(*p)).collect(),
                style: board::GraphicStyle::Silkscreen,
                closed: path.closed,
                stroke_width: None,
            }],
            "",
            at(primitive.anchor()),
            board::copper_color(Some(true)),
            selected,
            false,
        ),
    }
    if selected {
        painter.rect_stroke(
            bounds(primitive, camera, area).expand(authoring::PICK_PADDING),
            0,
            authoring::selection_stroke(),
            egui::StrokeKind::Outside,
        );
    }
}

/// Hit graphics on their ink, so an unfilled fabrication rectangle cannot intercept clicks
/// inside copper lands merely because its bounding box encloses them.
fn hit(primitive: &Primitive, camera: &crate::canvas_camera::Camera, area: Rect, pointer: egui::Pos2) -> bool {
    let at = |point| camera.screen(point, area);
    let scale = camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32;
    let near = |a: egui::Pos2, b: egui::Pos2| {
        let line = b - a;
        let length = line.length_sq();
        let fraction = if length > 0.0 { ((pointer - a).dot(line) / length).clamp(0.0, 1.0) } else { 0.0 };
        pointer.distance(a + line * fraction) <= authoring::PICK_PADDING
    };
    let rectangle = |center: Point, w: i64, h: i64, rotation: u16, filled: bool| {
        let offset = pointer - at(center);
        let angle = -(rotation as f32).to_radians();
        let local = vec2(offset.x * angle.cos() - offset.y * angle.sin(), offset.x * angle.sin() + offset.y * angle.cos());
        let extent = vec2(w as f32, h as f32) * scale / 2.0;
        if filled {
            local.x.abs() <= extent.x + authoring::PAD_PICK_PADDING && local.y.abs() <= extent.y + authoring::PAD_PICK_PADDING
        } else {
            let inside = local.x.abs() <= extent.x + authoring::PICK_PADDING && local.y.abs() <= extent.y + authoring::PICK_PADDING;
            inside && ((local.x.abs() - extent.x).abs() <= authoring::PICK_PADDING || (local.y.abs() - extent.y).abs() <= authoring::PICK_PADDING)
        }
    };
    match primitive {
        Primitive::Pin { at: point, .. } => pointer.distance(at(*point)) <= authoring::PIN_PICK_RADIUS,
        Primitive::Text { .. } => bounds(primitive, camera, area).expand(authoring::TEXT_PICK_PADDING).contains(pointer),
        Primitive::Line { from, to } => near(at(*from), at(*to)),
        Primitive::Rectangle { min, max } => {
            let a = at(*min);
            let b = at(*max);
            [a, egui::pos2(b.x, a.y), b, egui::pos2(a.x, b.y), a].windows(2).any(|pair| near(pair[0], pair[1]))
        }
        Primitive::Circle { center, radius_nm } => (pointer.distance(at(*center)) - *radius_nm as f32 * scale).abs() <= authoring::PICK_PADDING,
        Primitive::Polyline { points } => points.windows(2).any(|pair| near(at(pair[0]), at(pair[1]))),
        Primitive::Pad { pad } => rectangle(pad.center, pad.width_nm, pad.height_nm, pad.rotation, true),
        Primitive::Outline { graphic } => rectangle(graphic.center, graphic.width_nm, graphic.height_nm, graphic.rotation, false),
        Primitive::Silkscreen { path } => {
            path.points.windows(2).any(|pair| near(at(pair[0]), at(pair[1])))
                || (path.closed && path.points.first().zip(path.points.last()).is_some_and(|(a, b)| near(at(*a), at(*b))))
        }
    }
}
