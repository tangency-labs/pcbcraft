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
    Arc,
    Polyline,
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
    MoveGroup {
        objects: Vec<u64>,
        originals: Vec<Primitive>,
        anchor: Point,
        revision: u64,
    },
    Paste {
        primitives: Vec<Primitive>,
        anchor: Point,
        revision: u64,
    },
    Box {
        start: egui::Pos2,
        initial: std::collections::BTreeSet<u64>,
    },
    Polyline {
        points: Vec<Point>,
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
        Primitive::Graphic { graphic } => graphic_name(graphic).into(),
        Primitive::Artwork { artwork } => format!("{:?} {}", artwork.purpose, graphic_name(&artwork.graphic).to_lowercase()),
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
/// Keep geometry names shared between symbol drawing and physical layer-qualified artwork.
fn graphic_name(graphic: &pcbcraft_engine::library_graphics::Graphic) -> &'static str {
    use pcbcraft_engine::library_graphics::Geometry;
    match graphic.geometry {
        Geometry::Line { .. } => "Line",
        Geometry::Rectangle { .. } => "Rectangle",
        Geometry::Circle { .. } => "Circle",
        Geometry::Arc { .. } => "Arc",
        Geometry::Text { .. } => "Text",
        Geometry::Polyline { .. } => "Polyline",
    }
}
/// Extents include rotated copper corners and technical drawing ink rather than just anchors.
fn points(primitive: &Primitive) -> Vec<Point> {
    let rectangle = |center: Point, w: i64, h: i64, rotation: u16| {
        let a = f64::from(rotation).to_radians();
        [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]
            .into_iter()
            .map(|(x, y)| {
                let (x, y) = (x * w as f64, y * h as f64);
                Point {
                    x_nm: center.x_nm + (x * a.cos() - y * a.sin()).round() as i64,
                    y_nm: center.y_nm + (x * a.sin() + y * a.cos()).round() as i64,
                }
            })
            .collect()
    };
    match primitive {
        Primitive::Graphic { graphic } | Primitive::Artwork { artwork: foot::Artwork { graphic, .. } } => {
            graphic.paths().into_iter().flatten().collect()
        }
        Primitive::Pin { at, number, name, appearance: Some(a), .. } => {
            std::iter::once(*at).chain(a.graphics(*at, number, name.as_deref()).iter().flat_map(|g| g.paths().into_iter().flatten())).collect()
        }
        Primitive::Pin { at, .. } | Primitive::Text { at, .. } => vec![*at],
        Primitive::Line { from, to } => vec![*from, *to],
        Primitive::Rectangle { min, max } => vec![*min, *max],
        Primitive::Circle { center, radius_nm } => vec![
            Point { x_nm: center.x_nm - radius_nm, y_nm: center.y_nm - radius_nm },
            Point { x_nm: center.x_nm + radius_nm, y_nm: center.y_nm + radius_nm },
        ],
        Primitive::Polyline { points } => points.clone(),
        Primitive::Pad { pad } => rectangle(pad.center, pad.width_nm, pad.height_nm, pad.rotation),
        Primitive::Outline { graphic } => rectangle(graphic.center, graphic.width_nm, graphic.height_nm, graphic.rotation),
        Primitive::Silkscreen { path } => path.points.clone(),
    }
}
impl Document {
    /// Start a tool without discarding fields, inheriting the selected pin/pad's physical defaults.
    fn start_tool(&mut self, kind: Place) {
        if self.pending() {
            self.error = Some("Apply or reset the property fields before changing tools.".into());
            return;
        }
        let item = self.session.snapshot();
        if let Some(primitive) = self.selected.and_then(|id| item.objects.get(&id)).filter(|p| {
            matches!(
                (kind, *p),
                (Place::Pin, Primitive::Pin { .. })
                    | (Place::Pad, Primitive::Pad { .. })
                    | (
                        Place::Text,
                        Primitive::Graphic {
                            graphic: pcbcraft_engine::library_graphics::Graphic {
                                geometry: pcbcraft_engine::library_graphics::Geometry::Text { .. },
                                ..
                            }
                        }
                    )
                    | (
                        Place::Text,
                        Primitive::Artwork {
                            artwork: foot::Artwork {
                                graphic: pcbcraft_engine::library_graphics::Graphic {
                                    geometry: pcbcraft_engine::library_graphics::Geometry::Text { .. },
                                    ..
                                },
                                ..
                            }
                        }
                    )
            )
        }) {
            self.prototype = Some(primitive.clone());
        }
        self.tool = Tool::Draw { kind, start: None, revision: self.session.revision() };
        self.error = None;
    }
    /// Keep drawing, modification, clipboard/arrays and physical drawing defaults on the shared ribbon.
    pub(super) fn tools(&mut self, ui: &mut Ui) {
        use workspace::{RibbonTool, ToolIcon as Icon};
        let item = self.session.snapshot();
        let enabled = !self.pending();
        let selected = enabled && !self.selection.is_empty();
        let tools = [
            (None, "Select", Icon::Select),
            (
                Some(if item.kind == Kind::Symbol { Place::Pin } else { Place::Pad }),
                if item.kind == Kind::Symbol { "Pin" } else { "Pad" },
                if item.kind == Kind::Symbol { Icon::Pin } else { Icon::Pad },
            ),
            (Some(Place::Line), "Line", Icon::Line),
            (Some(Place::Rectangle), "Rectangle", Icon::Rectangle),
            (Some(Place::Circle), "Circle", Icon::Circle),
            (Some(Place::Arc), "Arc", Icon::Circle),
            (Some(Place::Polyline), "Polyline", Icon::Line),
            (Some(Place::Text), "Text", Icon::Text),
        ];
        let buttons = tools
            .iter()
            .map(|(kind, label, icon)| RibbonTool {
                icon: *icon,
                label,
                description: label,
                shortcut: "",
                enabled,
                active: match (&self.tool, kind) {
                    (Tool::Select, None) => true,
                    (Tool::Draw { kind: a, .. }, Some(b)) => a == b,
                    (Tool::Polyline { .. }, Some(Place::Polyline)) => true,
                    _ => false,
                },
                toggle: true,
            })
            .collect::<Vec<_>>();
        if let Some(index) = workspace::ribbon_group(ui, "Draw", &buttons) {
            if let Some(kind) = tools[index].0 {
                self.start_tool(kind);
            } else {
                self.tool = Tool::Select;
            }
        }
        workspace::ribbon_separator(ui);
        let button =
            |icon, label, available| RibbonTool { icon, label, description: label, shortcut: "", enabled: available, active: false, toggle: false };
        let tools = [
            button(Icon::Move, "Move", selected),
            button(Icon::Rotate, "Rotate", selected),
            button(Icon::Mirror, "Mirror X", selected),
            button(Icon::Delete, "Delete", selected),
        ];
        let extra = [button(Icon::Mirror, "Mirror Y", selected), button(Icon::Fit, "Set origin", enabled && self.selected.is_some())];
        if let Some(index) = workspace::ribbon_group_with_menu(ui, "Modify", &tools, &extra) {
            match index {
                0 => self.begin_move(),
                1 => self.rotate(),
                2 => self.transform_selection(0, true, false),
                3 => self.delete(),
                4 => self.transform_selection(0, false, true),
                _ => self.set_origin(),
            }
        }
        workspace::ribbon_separator(ui);
        let tools = [button(Icon::Sheet, "Duplicate", selected), button(Icon::Nets, "Array", selected)];
        let extra = [button(Icon::Sheet, "Copy", selected), button(Icon::Sheet, "Paste", enabled)];
        if let Some(index) = workspace::ribbon_group_with_menu(ui, "Copy", &tools, &extra) {
            match index {
                0 => self.duplicate(),
                1 => self.array_requested = true,
                2 => self.copy(ui.ctx()),
                _ => self.paste(ui.ctx()),
            }
        }
        workspace::ribbon_separator(ui);
        workspace::ribbon_control(
            ui,
            if item.kind == Kind::Footprint { "Layer / stroke" } else { "Fill / stroke" },
            Tokens::get().space_24 * 7.,
            |ui| {
                if item.kind == Kind::Footprint {
                    let title = match self.drawing_purpose {
                        GraphicPurpose::Fabrication => "Fabrication",
                        GraphicPurpose::Silkscreen => "Silkscreen",
                        GraphicPurpose::Courtyard => "Courtyard",
                    };
                    egui::ComboBox::from_id_salt("drawing_layer").selected_text(title).show_ui(ui, |ui| {
                        for (value, label) in [
                            (GraphicPurpose::Fabrication, "Fabrication"),
                            (GraphicPurpose::Silkscreen, "Silkscreen"),
                            (GraphicPurpose::Courtyard, "Courtyard"),
                        ] {
                            ui.selectable_value(&mut self.drawing_purpose, value, label);
                        }
                    });
                } else {
                    use pcbcraft_engine::library_graphics::Fill;
                    egui::ComboBox::from_id_salt("drawing_fill")
                        .selected_text(match self.drawing_fill {
                            Fill::None => "Unfilled",
                            Fill::Solid => "Solid fill",
                            Fill::Background => "Background fill",
                        })
                        .show_ui(ui, |ui| {
                            for (value, label) in [(Fill::None, "Unfilled"), (Fill::Solid, "Solid fill"), (Fill::Background, "Background fill")] {
                                ui.selectable_value(&mut self.drawing_fill, value, label);
                            }
                        });
                }
                let mut width = self.drawing_stroke_nm as f64 / 1_000_000.;
                if ui.add(egui::DragValue::new(&mut width).range(0.01..=2.).speed(0.01).suffix(" mm stroke")).changed() {
                    self.drawing_stroke_nm = (width * 1_000_000.).round() as i64;
                }
            },
        );
    }
    /// Navigation and panel visibility stay local to this definition, as do its grid and camera.
    pub(super) fn view_tools(&mut self, ui: &mut Ui) {
        use workspace::{RibbonTool, ToolIcon as Icon};
        let tools = [(Icon::Fit, "Fit", false), (Icon::Browser, "Browser", !self.collapsed[0]), (Icon::Settings, "Properties", !self.collapsed[1])]
            .map(|(icon, label, active)| RibbonTool { icon, label, description: label, shortcut: "", enabled: true, active, toggle: label != "Fit" });
        if let Some(index) = workspace::ribbon_group(ui, "View", &tools) {
            match index {
                0 => self.camera.request_fit(),
                index => self.collapsed[index - 1] ^= true,
            }
        }
        workspace::ribbon_separator(ui);
        workspace::ribbon_control(ui, "Grid", Tokens::get().space_24 * 5.0, |ui| {
            egui::ComboBox::from_id_salt("library_grid").selected_text(format!("Grid {} mm", self.camera.snap_nm() as f64 / 1_000_000.0)).show_ui(
                ui,
                |ui| {
                    let spacings: &[i64] = if self.session.snapshot().kind == Kind::Symbol {
                        &[1_270_000]
                    } else {
                        &[25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000]
                    };
                    for &spacing in spacings {
                        if ui.selectable_label(self.camera.snap_nm() == spacing, format!("{} mm", spacing as f64 / 1_000_000.0)).clicked() {
                            self.camera.set_snap_nm(spacing);
                        }
                    }
                },
            );
        });
    }
    /// Move retains the captured selection and revision until a completed placement.
    fn begin_move(&mut self) {
        if self.pending() {
            self.error = Some("Apply or reset the property fields before moving.".into());
            return;
        }
        let item = self.session.snapshot();
        if self.selection.len() == 1
            && let Some(object) = self.selected
            && let Some(original) = item.objects.get(&object)
        {
            self.tool = Tool::Move { object, original: original.clone(), revision: self.session.revision() };
        } else {
            let objects: Vec<_> = self.selection.iter().copied().collect();
            let originals: Vec<_> = objects.iter().filter_map(|id| item.objects.get(id).cloned()).collect();
            if let Some(anchor) = originals.first().map(Primitive::anchor) {
                self.tool = Tool::MoveGroup { objects, originals, anchor, revision: self.session.revision() };
            }
        }
    }
    /// Delete the complete stable selection, allowing an unfinished draft to have no pins/pads.
    fn delete(&mut self) {
        if !self.selection.is_empty()
            && let Err(error) = self.execute(self.session.revision(), Action::Delete { objects: self.selection.iter().copied().collect() })
        {
            self.error = Some(error);
        }
    }
    /// R rotates the placement prototype or applies a validated rotation to the selected objects.
    fn rotate(&mut self) {
        self.rotate_by(-1);
    }
    /// Placement rotation stays in the cancellable preview until the move/paste is committed.
    fn rotate_by(&mut self, turns: i16) {
        let preview = match &mut self.tool {
            Tool::Move { original, .. } => Some((vec![original.clone()], original.anchor())),
            Tool::MoveGroup { originals, anchor, .. } => Some((originals.clone(), *anchor)),
            Tool::Paste { primitives, anchor, .. } => Some((primitives.clone(), *anchor)),
            _ => None,
        };
        if let Some((originals, anchor)) = preview {
            let t = pcbcraft_engine::library_graphics::Transform {
                pivot: anchor,
                offset: Point { x_nm: 0, y_nm: 0 },
                quarter_turns: turns,
                mirror_x: false,
                mirror_y: false,
            };
            match originals.iter().map(|p| p.transformed(t)).collect::<Result<Vec<_>, _>>() {
                Ok(mut transformed) => match &mut self.tool {
                    Tool::Move { original, .. } => {
                        if let Some(p) = transformed.pop() {
                            *original = p;
                        }
                    }
                    Tool::MoveGroup { originals, .. } => *originals = transformed,
                    Tool::Paste { primitives, .. } => *primitives = transformed,
                    _ => {}
                },
                Err(error) => self.error = Some(error),
            }
            return;
        }
        if let Tool::Draw { kind, .. } = self.tool {
            let item = self.session.snapshot();
            if let Ok(primitive) = self.placement_primitive(&item, kind, Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 0, y_nm: 0 }) {
                let t = pcbcraft_engine::library_graphics::Transform {
                    pivot: primitive.anchor(),
                    offset: Point { x_nm: 0, y_nm: 0 },
                    quarter_turns: turns,
                    mirror_x: false,
                    mirror_y: false,
                };
                if let Ok(primitive) = primitive.transformed(t) {
                    self.prototype = Some(primitive);
                }
            }
        } else {
            self.transform_selection(turns, false, false);
        }
    }
    /// Repeated pins and pads inherit dimensions/type/orientation from the selected prototype.
    fn placement_primitive(&self, item: &Item, kind: Place, start: Point, end: Point) -> Result<Primitive, String> {
        let prototype = self.prototype.as_ref().filter(|p| {
            matches!(
                (kind, *p),
                (Place::Pin, Primitive::Pin { .. })
                    | (Place::Pad, Primitive::Pad { .. })
                    | (
                        Place::Text,
                        Primitive::Graphic {
                            graphic: pcbcraft_engine::library_graphics::Graphic {
                                geometry: pcbcraft_engine::library_graphics::Geometry::Text { .. },
                                ..
                            }
                        }
                    )
                    | (
                        Place::Text,
                        Primitive::Artwork {
                            artwork: foot::Artwork {
                                graphic: pcbcraft_engine::library_graphics::Graphic {
                                    geometry: pcbcraft_engine::library_graphics::Geometry::Text { .. },
                                    ..
                                },
                                ..
                            }
                        }
                    )
            )
        });
        let unused_number = |seed: &str| {
            if item.objects.values().any(|p| match p {
                Primitive::Pin { number, .. } => number == seed,
                Primitive::Pad { pad } => pad.number == seed,
                _ => false,
            }) {
                pcbcraft_engine::authoring::next_number(item, seed)
            } else {
                Ok(seed.to_owned())
            }
        };
        if let Some(prototype) = prototype {
            let mut p = prototype.translated(Point { x_nm: end.x_nm - prototype.anchor().x_nm, y_nm: end.y_nm - prototype.anchor().y_nm })?;
            match &mut p {
                Primitive::Pin { number, appearance, .. } => {
                    *number = unused_number(number)?;
                    if appearance.is_none() {
                        *appearance = Some(Default::default());
                    }
                }
                Primitive::Pad { pad } => pad.number = unused_number(&pad.number)?,
                _ => {}
            }
            return Ok(p);
        }
        use pcbcraft_engine::library_graphics::{Fill, Geometry, Graphic};
        let geometry = match kind {
            Place::Line => Some(Geometry::Line { from: start, to: end }),
            Place::Rectangle => Some(Geometry::Rectangle {
                min: Point { x_nm: start.x_nm.min(end.x_nm), y_nm: start.y_nm.min(end.y_nm) },
                max: Point { x_nm: start.x_nm.max(end.x_nm), y_nm: start.y_nm.max(end.y_nm) },
            }),
            Place::Circle | Place::Arc => {
                let radius_nm = ((end.x_nm - start.x_nm) as f64).hypot((end.y_nm - start.y_nm) as f64).round() as i64;
                Some(if kind == Place::Circle {
                    Geometry::Circle { center: start, radius_nm }
                } else {
                    Geometry::Arc { center: start, radius_nm, start_degrees: 0, sweep_degrees: 180 }
                })
            }
            Place::Text => Some(Geometry::Text { at: end, text: "Text".into(), height_nm: 1_000_000, rotation: 0, align: TextAlign::Start }),
            Place::Polyline => Some(Geometry::Polyline { points: vec![start, end], closed: false }),
            _ => None,
        };
        if let Some(geometry) = geometry {
            let mut graphic = Graphic { geometry, stroke_nm: self.drawing_stroke_nm, fill: Fill::None };
            if graphic.closed() && item.kind == Kind::Symbol {
                graphic.fill = self.drawing_fill;
            }
            return Ok(if item.kind == Kind::Symbol {
                Primitive::Graphic { graphic }
            } else {
                Primitive::Artwork { artwork: foot::Artwork { purpose: self.drawing_purpose, graphic } }
            });
        }
        primitive(item, kind, start, end)
    }
    /// Navigate and edit the focused pane through revision-checked transactions. Directional
    /// boxes, modifier selection, clipboard placement and repeated drawing share stable IDs.
    pub(super) fn canvas(&mut self, ui: &mut Ui, area: Rect, covered: [Rect; 2], navigation: Navigation, active: bool) {
        use pcbcraft_engine::library_graphics::{Fill, Geometry, Graphic};
        let item = self.session.snapshot();
        let t = Tokens::get();
        let response = ui.interact(area, ui.id().with("library_canvas"), egui::Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), "Library canvas"));
        if !ui.input(|i| i.focused) {
            self.tool = Tool::Select;
        }
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = active
            && ui.is_enabled()
            && !egui::Popup::is_any_open(ui.ctx())
            && pointer.is_some_and(|p| {
                area.contains(p)
                    && !covered.iter().any(|r| r.contains(p))
                    && ui.ctx().layer_id_at(p).is_none_or(|layer| layer.order == egui::Order::Background)
            });
        self.camera.set_uncovered(area, workspace::catalog::uncovered(area, &covered));
        self.camera.navigate_definition(&response, area, item.objects.values().flat_map(points), over, navigation);
        let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(area);
        painter.rect_filled(area, 0, if item.kind == Kind::Symbol { t.canvas } else { t.board_canvas });
        let origin = self.camera.screen(Point { x_nm: 0, y_nm: 0 }, area);
        grid::paint(&painter, area, origin, self.camera.snap_step(), authoring::GRID_OPACITY);
        let drawing_fill = self.drawing_fill;
        let graphic_object = |points: Vec<Point>, closed: bool, purpose, stroke_nm| {
            let graphic = Graphic {
                geometry: Geometry::Polyline { points, closed },
                stroke_nm,
                fill: if closed && item.kind == Kind::Symbol { drawing_fill } else { Fill::None },
            };
            if item.kind == Kind::Symbol {
                Primitive::Graphic { graphic }
            } else {
                Primitive::Artwork { artwork: foot::Artwork { purpose, graphic } }
            }
        };
        if over && response.drag_started() && matches!(self.tool, Tool::Select) && !self.pending() {
            let start = ui.input(|i| i.pointer.press_origin()).unwrap_or_else(|| pointer.unwrap_or(area.center()));
            let found = pick(&item, &self.camera, area, start);
            if let Some(id) = found {
                if !self.selection.contains(&id) {
                    let _ = self.select(Some(id));
                }
                let objects: Vec<_> = self.selection.iter().copied().collect();
                let originals = objects.iter().filter_map(|id| item.objects.get(id).cloned()).collect();
                self.tool = Tool::MoveGroup { objects, originals, anchor: self.camera.snap(start, area), revision: self.session.revision() };
            } else {
                self.tool = Tool::Box {
                    start,
                    initial: if ui.input(|i| i.modifiers.shift || i.modifiers.command) { self.selection.clone() } else { Default::default() },
                };
            }
        }
        let mut preview = Vec::new();
        if let Some(point) = pointer.filter(|_| over) {
            let at = self.camera.snap(point, area);
            preview = match &self.tool {
                Tool::Move { original, .. } => original
                    .translated(Point { x_nm: at.x_nm - original.anchor().x_nm, y_nm: at.y_nm - original.anchor().y_nm })
                    .into_iter()
                    .collect(),
                Tool::MoveGroup { originals, anchor, .. } | Tool::Paste { primitives: originals, anchor, .. } => {
                    originals.iter().filter_map(|p| p.translated(Point { x_nm: at.x_nm - anchor.x_nm, y_nm: at.y_nm - anchor.y_nm }).ok()).collect()
                }
                Tool::Draw { kind, start, .. } => self.placement_primitive(&item, *kind, start.unwrap_or(at), at).into_iter().collect(),
                Tool::Polyline { points, .. } => {
                    let mut points = points.clone();
                    if points.last() != Some(&at) {
                        points.push(at);
                    }
                    vec![graphic_object(points, false, self.drawing_purpose, self.drawing_stroke_nm)]
                }
                _ => Vec::new(),
            };
            if response.clicked() || response.drag_stopped() {
                let result = match self.tool.clone() {
                    Tool::Select => {
                        let found = pick(&item, &self.camera, area, point);
                        let mut ids =
                            if ui.input(|i| i.modifiers.shift || i.modifiers.command) { self.selection.clone() } else { Default::default() };
                        if let Some(id) = found
                            && !ids.insert(id)
                        {
                            ids.remove(&id);
                        }
                        let result = self.select_many(ids);
                        if response.double_clicked() && found.is_some() {
                            self.details_requested = true;
                        }
                        result
                    }
                    Tool::Move { object, revision, .. } => preview
                        .first()
                        .cloned()
                        .ok_or("No move preview.".into())
                        .and_then(|primitive| self.execute(revision, Action::Replace { object, primitive })),
                    Tool::MoveGroup { objects, originals, anchor, revision } => originals
                        .into_iter()
                        .map(|p| p.translated(Point { x_nm: at.x_nm - anchor.x_nm, y_nm: at.y_nm - anchor.y_nm }))
                        .collect::<Result<Vec<_>, _>>()
                        .and_then(|primitives| {
                            self.execute(revision, Action::ReplaceMany { primitives: objects.into_iter().zip(primitives).collect() })
                        }),
                    Tool::Paste { primitives, anchor, revision } => self
                        .execute(revision, Action::Paste { primitives, offset: Point { x_nm: at.x_nm - anchor.x_nm, y_nm: at.y_nm - anchor.y_nm } }),
                    Tool::Box { start, mut initial } => {
                        let rect = Rect::from_two_pos(start, point);
                        let crossing = point.x < start.x;
                        initial.extend(
                            item.objects
                                .iter()
                                .filter(|(_, p)| {
                                    let b = bounds(p, &self.camera, area);
                                    if crossing { rect.intersects(b) } else { rect.contains_rect(b) }
                                })
                                .map(|(id, _)| *id),
                        );
                        let result = self.select_many(initial);
                        self.tool = Tool::Select;
                        result
                    }
                    Tool::Draw { kind: Place::Polyline, revision, .. } => {
                        self.tool = Tool::Polyline { points: vec![at], revision };
                        Ok(())
                    }
                    Tool::Polyline { mut points, revision } => {
                        let closed = points.len() >= 3 && points.first() == Some(&at);
                        if points.last() != Some(&at) && !closed {
                            points.push(at);
                        }
                        if closed || response.double_clicked() {
                            let result = self.execute(
                                revision,
                                Action::Insert { primitive: graphic_object(points, closed, self.drawing_purpose, self.drawing_stroke_nm) },
                            );
                            if result.is_ok() {
                                self.tool = Tool::Draw { kind: Place::Polyline, start: None, revision: self.session.revision() };
                            }
                            result
                        } else {
                            self.tool = Tool::Polyline { points, revision };
                            Ok(())
                        }
                    }
                    Tool::Draw { kind, start: None, revision } if matches!(kind, Place::Line | Place::Rectangle | Place::Circle | Place::Arc) => {
                        self.tool = Tool::Draw { kind, start: Some(at), revision };
                        Ok(())
                    }
                    Tool::Draw { kind, start, revision } => self.placement_primitive(&item, kind, start.unwrap_or(at), at).and_then(|primitive| {
                        let prototype = primitive.clone();
                        self.execute(revision, Action::Insert { primitive })?;
                        if matches!(kind, Place::Pin | Place::Pad | Place::Text) {
                            self.prototype = Some(prototype);
                        }
                        self.tool = Tool::Draw { kind, start: None, revision: self.session.revision() };
                        Ok(())
                    }),
                };
                if let Err(error) = result {
                    self.error = Some(error);
                    self.tool = Tool::Select;
                }
            }
        } else if response.drag_stopped() {
            self.tool = Tool::Select;
        }
        if active && ui.is_enabled() && !ui.ctx().egui_wants_keyboard_input() {
            let command = ui.input(|i| i.modifiers.command);
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.tool = Tool::Select;
            }
            if over {
                if command && ui.input(|i| i.key_pressed(egui::Key::A)) {
                    let _ = self.select_many(item.objects.keys().copied().collect());
                }
                if command && ui.input(|i| i.key_pressed(egui::Key::C)) {
                    self.copy(ui.ctx());
                }
                if command && ui.input(|i| i.key_pressed(egui::Key::V)) {
                    self.paste(ui.ctx());
                }
                if command && ui.input(|i| i.key_pressed(egui::Key::D)) {
                    self.duplicate();
                }
                if !command && ui.input(|i| i.key_pressed(egui::Key::P)) {
                    self.start_tool(if item.kind == Kind::Symbol { Place::Pin } else { Place::Pad });
                }
                if !command && ui.input(|i| i.key_pressed(egui::Key::L)) {
                    self.start_tool(Place::Line);
                }
                if !command && ui.input(|i| i.key_pressed(egui::Key::T)) {
                    self.start_tool(Place::Text);
                }
                if ui.input(|i| i.key_pressed(egui::Key::M)) {
                    self.begin_move();
                }
                if ui.input(|i| i.key_pressed(egui::Key::R)) {
                    self.rotate_by(if ui.input(|i| i.modifiers.shift) { 1 } else { -1 });
                }
                if ui.input(|i| i.key_pressed(egui::Key::E)) && self.selection.len() == 1 {
                    self.details_requested = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    self.delete();
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Tool::Polyline { points, revision } = self.tool.clone()
                    && let Err(error) = self
                        .execute(revision, Action::Insert { primitive: graphic_object(points, false, self.drawing_purpose, self.drawing_stroke_nm) })
                {
                    self.error = Some(error);
                }
            }
        }
        // Body fills stay behind terminal labels regardless of insertion order.
        let filled = |p: &&Primitive| matches!(p, Primitive::Graphic { graphic } if graphic.fill != Fill::None);
        let objects = item.objects.iter().filter(|(_, p)| filled(p)).chain(item.objects.iter().filter(|(_, p)| !filled(p)));
        for (id, p) in objects {
            let moving = matches!(&self.tool,Tool::Move {object,..} if object==id)
                || matches!(&self.tool,Tool::MoveGroup {objects,..} if objects.contains(id));
            if !moving || preview.is_empty() {
                paint(&painter, p, &self.camera, area, self.selection.contains(id));
            }
        }
        for p in preview {
            paint(&painter, &p, &self.camera, area, true);
        }
        if let Tool::Box { start, .. } = self.tool
            && let Some(point) = pointer
        {
            let rect = Rect::from_two_pos(start, point);
            let crossing = point.x < start.x;
            let count = item
                .objects
                .values()
                .filter(|p| {
                    let b = bounds(p, &self.camera, area);
                    if crossing { rect.intersects(b) } else { rect.contains_rect(b) }
                })
                .count();
            pcbcraft_chrome::selection::marquee(&painter, rect, crossing, count);
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
        Place::Pin => Primitive::Pin { number, name: None, electrical_type: ElectricalType::Passive, at: end, appearance: Some(Default::default()) },
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
        Place::Arc | Place::Polyline => return Err("Choose drawing geometry through its parameterized tool.".into()),
        Place::Line => Primitive::Line { from: start, to: end },
        Place::Rectangle => Primitive::Rectangle { min, max },
        Place::Circle => {
            Primitive::Circle { center: start, radius_nm: ((end.x_nm - start.x_nm) as f64).hypot((end.y_nm - start.y_nm) as f64).round() as i64 }
        }
        Place::Text => Primitive::Text { at: end, text: "Text".into(), align: TextAlign::Start },
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
        Primitive::Graphic { graphic } => crate::definition_graphics::paint(painter, graphic, at, scale, t.canvas_ink, t.canvas),
        Primitive::Artwork { artwork } => {
            let ink = match artwork.purpose {
                GraphicPurpose::Silkscreen => t.board_ink,
                GraphicPurpose::Fabrication => t.text_dim,
                GraphicPurpose::Courtyard => t.icon_active_2,
            };
            crate::definition_graphics::paint(painter, &artwork.graphic, at, scale, ink, t.board_canvas);
        }
        Primitive::Pin { at: point, number, name, appearance: Some(a), .. } => {
            for graphic in a.graphics(*point, number, name.as_deref()) {
                crate::definition_graphics::paint(painter, &graphic, at, scale, t.canvas_ink, t.canvas);
            }
            if selected {
                painter.circle_stroke(at(*point), authoring::PIN_RADIUS, authoring::selection_stroke());
            }
        }
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
                color: Some(if pad.copper == CopperBinding::AllCopper { Tokens::get().board_through } else { board::copper_color(Some(true)) }),
                number: pad.number.clone(),
                center: at(pad.center),
                copper: Some(corners(pad.center, pad.width_nm, pad.height_nm, pad.rotation)),
                contour: match pad.shape {
                    PadShape::Rectangle => board::PadContour::Rectangle,
                    PadShape::Circle => board::PadContour::Circle,
                    PadShape::Oval => board::PadContour::Oval,
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
                color: None,
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
                color: None,
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
        Primitive::Graphic { graphic } | Primitive::Artwork { artwork: foot::Artwork { graphic, .. } } => {
            graphic.paths().iter().any(|points| {
                points.windows(2).any(|p| near(at(p[0]), at(p[1])))
                    || (graphic.closed() && points.first().zip(points.last()).is_some_and(|(a, b)| near(at(*a), at(*b))))
            }) || (graphic.fill != pcbcraft_engine::library_graphics::Fill::None && bounds(primitive, camera, area).contains(pointer))
        }
        Primitive::Pin { at: point, number, name, appearance: Some(a), .. } => {
            pointer.distance(at(*point)) <= authoring::PIN_PICK_RADIUS
                || near(at(*point), at(a.end(*point)))
                || a.graphics(*point, number, name.as_deref()).iter().flat_map(|g| g.paths()).any(|p| p.windows(2).any(|p| near(at(p[0]), at(p[1]))))
        }
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

/// Electrical objects win overlaps with body graphics so labels and lands remain directly editable.
fn pick(item: &Item, camera: &crate::canvas_camera::Camera, area: Rect, at: egui::Pos2) -> Option<u64> {
    item.objects
        .iter()
        .rev()
        .filter(|(_, p)| matches!(p, Primitive::Pin { .. } | Primitive::Pad { .. }))
        .chain(item.objects.iter().rev().filter(|(_, p)| !matches!(p, Primitive::Pin { .. } | Primitive::Pad { .. })))
        .find(|(_, p)| hit(p, camera, area, at))
        .map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Rotating an uncommitted pin preview must not consume or skip an electrical number.
    #[test]
    fn placement_rotation_keeps_the_next_number_and_selected_defaults() {
        let mut editors = super::super::Editors::default();
        let id = editors.open(Item::new(Kind::Symbol).unwrap(), false).unwrap();
        let doc = editors.documents.get_mut(&id).unwrap();
        doc.select(Some(1)).unwrap();
        doc.start_tool(Place::Pin);
        doc.rotate_by(1);
        doc.rotate_by(-1);
        let item = doc.session.snapshot();
        let at = Point { x_nm: -5_080_000, y_nm: 2_540_000 };
        let primitive = doc.placement_primitive(&item, Place::Pin, at, at).unwrap();
        let Primitive::Pin { number, appearance, .. } = &primitive else { panic!() };
        assert_eq!(number, "3");
        assert_eq!(appearance.as_ref().unwrap().orientation, pcbcraft_engine::schematic::PinOrientation::Right);
        assert_eq!(doc.session.revision(), 0);
        doc.execute(0, Action::Insert { primitive }).unwrap();
        assert_eq!(doc.session.snapshot().symbol().unwrap().pins().len(), 3);
    }
    /// Selecting a drawing cannot turn the Text tool into a rectangle repetition tool.
    #[test]
    fn text_placement_and_closed_edge_picking_follow_geometry() {
        use pcbcraft_engine::library_graphics::{Fill, Geometry, Graphic};
        let mut editors = super::super::Editors::default();
        let id = editors.open(Item::new(Kind::Symbol).unwrap(), false).unwrap();
        let doc = editors.documents.get_mut(&id).unwrap();
        let origin = Point { x_nm: 0, y_nm: 0 };
        let g = Graphic {
            geometry: Geometry::Rectangle { min: origin, max: Point { x_nm: 2_540_000, y_nm: 2_540_000 } },
            stroke_nm: 100_000,
            fill: Fill::None,
        };
        doc.execute(0, Action::Insert { primitive: Primitive::Graphic { graphic: g.clone() } }).unwrap();
        doc.start_tool(Place::Text);
        assert!(matches!(
            doc.placement_primitive(&doc.session.snapshot(), Place::Text, origin, origin).unwrap(),
            Primitive::Graphic { graphic: Graphic { geometry: Geometry::Text { .. }, .. } }
        ));
        let area = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
        let point = doc.camera.screen(Point { x_nm: 0, y_nm: 1_270_000 }, area);
        assert!(hit(&Primitive::Graphic { graphic: g }, &doc.camera, area, point));
    }
}
