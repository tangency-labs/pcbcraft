//! Original canvas tools project native primitives through existing Craft painters.
use super::*;
use crate::path_edit::{Canvas as PathCanvas, Drag as PathDrag, VertexAction};
use egui::{Rect, vec2};
use pcbcraft_chrome::menu::CheckKind;
use pcbcraft_chrome::{Tokens, board, grid, workspace};
use pcbcraft_engine::{
    authoring::Primitive,
    board::definition::{self as foot, CopperBinding, GraphicPurpose, PadShape},
    schematic::{ElectricalType, Point, TextAlign},
};

mod paths;
use paths::DefinitionPath;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Place {
    Pin,
    Pad,
    NonPlatedHole,
    Line,
    Rectangle,
    Circle,
    Text,
    Arc,
    Polyline,
    Courtyard,
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
    Courtyard {
        points: Vec<Point>,
        revision: u64,
    },
    DragCourtyardVertex {
        object: u64,
        vertex: usize,
        original: Primitive,
        revision: u64,
    },
    EditPath {
        object: u64,
        drag: PathDrag,
        revision: u64,
    },
    VertexMenu {
        object: u64,
        revision: u64,
        action: VertexAction,
    },
    Move {
        object: u64,
        original: Primitive,
        revision: u64,
    },
}
/// Which cancellable preview an input batch's last preview chord resolved to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Preview {
    Paste,
    Duplicate,
}
/// Use the existing drawing-tool glyphs in Browser rows, without deriving identity from labels.
pub(super) fn browser_icon(primitive: &Primitive) -> pcbcraft_chrome::workspace::ToolIcon {
    use pcbcraft_chrome::workspace::ToolIcon;
    match primitive {
        Primitive::Pin { .. } => ToolIcon::Pin,
        Primitive::Pad { .. } => ToolIcon::Pad,
        Primitive::Rectangle { .. } | Primitive::Outline { .. } => ToolIcon::Rectangle,
        Primitive::Circle { .. } | Primitive::NonPlatedHole { .. } => ToolIcon::Circle,
        Primitive::Text { .. } | Primitive::Silkscreen { .. } => ToolIcon::Text,
        Primitive::Line { .. } | Primitive::Polyline { .. } => ToolIcon::Line,
        Primitive::Graphic { graphic } => graphic_icon(graphic),
        Primitive::Artwork { artwork } => graphic_icon(&artwork.graphic),
    }
}

/// Geometry uses the same glyph across schematic and layer-qualified definition artwork.
fn graphic_icon(graphic: &pcbcraft_engine::library_graphics::Graphic) -> pcbcraft_chrome::workspace::ToolIcon {
    use pcbcraft_chrome::workspace::ToolIcon;
    use pcbcraft_engine::library_graphics::Geometry;
    match graphic.geometry {
        Geometry::Line { .. } | Geometry::Polyline { .. } => ToolIcon::Line,
        Geometry::Rectangle { .. } => ToolIcon::Rectangle,
        Geometry::Circle { .. } | Geometry::Arc { .. } => ToolIcon::Circle,
        Geometry::Text { .. } => ToolIcon::Text,
    }
}

/// Human-readable browser labels keep electrical numbers visible beside stable primitive IDs.
pub(super) fn name(primitive: &Primitive) -> String {
    if pcbcraft_engine::authoring::courtyard::courtyard_polygon(primitive).is_some() {
        return "Courtyard outline".into();
    }
    match primitive {
        Primitive::Graphic { graphic } => graphic_name(graphic).into(),
        Primitive::Artwork { artwork } => format!("{:?} {}", artwork.purpose, graphic_name(&artwork.graphic).to_lowercase()),
        Primitive::Pin { number, .. } => format!("Pin {number}"),
        Primitive::Pad { pad } => format!("Pad {}", pad.number),
        Primitive::NonPlatedHole { .. } => "Non-plated hole".into(),
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
        Primitive::NonPlatedHole { hole } => rectangle(hole.center, hole.diameter_nm, hole.diameter_nm, 0),
        Primitive::Outline { graphic } => rectangle(graphic.center, graphic.width_nm, graphic.height_nm, graphic.rotation),
        Primitive::Silkscreen { path } => path.points.clone(),
    }
}
impl Document {
    /// Adapt an outline context action to the selected definition object and its own history.
    fn show_vertex_menu(&mut self, response: &egui::Response, item: &Item, area: Rect, over: bool, active: bool) {
        let popup = egui::Popup::default_response_id(response);
        if matches!(self.tool, Tool::VertexMenu { .. })
            && (!active || !egui::Popup::is_id_open(&response.ctx, popup) && !response.secondary_clicked())
        {
            self.tool = Tool::Select;
        }
        if over
            && response.secondary_clicked()
            && !self.pending()
            && matches!(self.tool, Tool::Select)
            && let Some(object) = self.selected
            && let Some(path) = item.objects.get(&object).and_then(DefinitionPath::of)
            && let Some(pointer) = response.interact_pointer_pos()
            && let Some(action) = VertexAction::at(PathCanvas { camera: &self.camera, rect: area }, pointer, path.outline(), 64, |points| {
                points.windows(2).all(|pair| pair[0] != pair[1]) && (!path.outline().closed || points.first() != points.last())
            })
        {
            self.tool = Tool::VertexMenu { object, revision: self.session.revision(), action };
        }
        if let Tool::VertexMenu { object, revision, action } = &self.tool {
            if *revision != self.session.revision() || self.selected != Some(*object) || self.pending() {
                egui::Popup::close_id(&response.ctx, popup);
                self.tool = Tool::Select;
                return;
            }
            let object = *object;
            let revision = *revision;
            if let Some(points) = pcbcraft_chrome::menu::context(response, |ui| action.show(ui)).and_then(|popup| popup.inner)
                && let Some(path) = item.objects.get(&object).and_then(DefinitionPath::of)
            {
                let result = self.execute(revision, Action::Replace { object, primitive: path.replace(points) });
                self.error = result.err();
                self.tool = Tool::Select;
            }
        }
    }

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
        let mut tools = vec![
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
        if item.kind == Kind::Footprint {
            tools.push((Some(Place::NonPlatedHole), "Non-plated hole", Icon::Circle));
        }
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
                menu_check: Some(CheckKind::Choice),
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
        let button = |icon, label, available| RibbonTool {
            icon,
            label,
            description: label,
            shortcut: "",
            enabled: available,
            active: false,
            menu_check: None,
        };
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
        let extra = [button(Icon::Sheet, "Copy", selected), button(Icon::Sheet, "Cut", selected), button(Icon::Sheet, "Paste", enabled)];
        if let Some(index) = workspace::ribbon_group_with_menu(ui, "Copy", &tools, &extra) {
            match index {
                0 => self.duplicate(),
                1 => self.array_requested = true,
                2 => self.copy(ui.ctx()),
                3 => self.cut(ui.ctx()),
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
                    pcbcraft_chrome::controls::Choice::from_id_salt("drawing_layer")
                        .selected_text(title)
                        .show_ui(ui, |ui| {
                            for (value, label) in [
                                (GraphicPurpose::Fabrication, "Fabrication"),
                                (GraphicPurpose::Silkscreen, "Silkscreen"),
                                (GraphicPurpose::Courtyard, "Courtyard"),
                            ] {
                                ui.selectable_value(&mut self.drawing_purpose, value, label);
                            }
                        })
                        .response
                        .accessible_name("Drawing layer");
                } else {
                    use pcbcraft_engine::library_graphics::Fill;
                    pcbcraft_chrome::controls::Choice::from_id_salt("drawing_fill")
                        .selected_text(match self.drawing_fill {
                            Fill::None => "Unfilled",
                            Fill::Solid => "Solid fill",
                            Fill::Background => "Background fill",
                        })
                        .show_ui(ui, |ui| {
                            for (value, label) in [(Fill::None, "Unfilled"), (Fill::Solid, "Solid fill"), (Fill::Background, "Background fill")] {
                                ui.selectable_value(&mut self.drawing_fill, value, label);
                            }
                        })
                        .response
                        .accessible_name("Drawing fill");
                }
                let mut width = self.drawing_stroke_nm as f64 / 1_000_000.;
                if ui
                    .add(egui::DragValue::new(&mut width).range(0.01..=2.).speed(0.01).suffix(" mm stroke"))
                    .accessible_name("Drawing stroke width")
                    .changed()
                {
                    self.drawing_stroke_nm = (width * 1_000_000.).round() as i64;
                }
            },
        );
        if item.kind == Kind::Footprint {
            workspace::ribbon_separator(ui);
            let courtyard_tools = [
                RibbonTool {
                    icon: Icon::Rectangle,
                    label: "Courtyard",
                    description: "Courtyard",
                    shortcut: "",
                    enabled,
                    active: matches!(self.tool, Tool::Draw { kind: Place::Courtyard, .. } | Tool::Courtyard { .. }),
                    menu_check: Some(CheckKind::Choice),
                },
                RibbonTool {
                    icon: Icon::Rules,
                    label: "Generate…",
                    description: "Generate courtyard outline from pads and body",
                    shortcut: "",
                    enabled,
                    active: false,
                    menu_check: None,
                },
            ];
            if let Some(index) = workspace::ribbon_group(ui, "Courtyard", &courtyard_tools) {
                match index {
                    0 => {
                        self.start_tool(Place::Courtyard);
                        if matches!(self.tool, Tool::Draw { kind: Place::Courtyard, .. }) {
                            self.set_show_courtyard(true);
                        }
                    }
                    _ => self.courtyard_requested = true,
                }
            }
        }
    }
    /// Navigation and panel visibility stay local to this definition, as do its grid and camera.
    pub(super) fn view_tools(&mut self, ui: &mut Ui) {
        use workspace::{RibbonTool, ToolIcon as Icon};
        let mut entries = vec![
            (Icon::Fit, "Fit", "Fit", false),
            (Icon::Browser, "Browser", "Browser", !self.collapsed[0]),
            (Icon::Settings, "Properties", "Properties", !self.collapsed[1]),
        ];
        if self.session.snapshot().kind == Kind::Footprint {
            entries.push((Icon::Rectangle, "Courtyard layer", "Courtyard layer", self.show_courtyard));
        }
        let tools: Vec<_> = entries
            .into_iter()
            .map(|(icon, label, description, active)| RibbonTool {
                icon,
                label,
                description,
                shortcut: "",
                enabled: true,
                active,
                menu_check: (label != "Fit").then_some(CheckKind::Toggle),
            })
            .collect();
        if let Some(index) = workspace::ribbon_group(ui, "View", &tools) {
            match index {
                0 => self.camera.request_fit(),
                1 => self.collapsed[0] ^= true,
                2 => self.collapsed[1] ^= true,
                _ => self.set_show_courtyard(!self.show_courtyard),
            }
        }
        workspace::ribbon_separator(ui);
        workspace::ribbon_control(ui, "Grid", Tokens::get().space_24 * 5.0, |ui| {
            pcbcraft_chrome::controls::Choice::from_id_salt("library_grid")
                .selected_text(format!("Grid {} mm", self.camera.snap_nm() as f64 / 1_000_000.0))
                .show_ui(ui, |ui| {
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
                })
                .response
                .accessible_name("Library grid");
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
    /// Cut stores the selection on the internal clipboard and removes it as one undoable edit.
    /// Pending fields refuse before changing the clipboard; an empty selection is a no-op.
    /// Deletion failures leave the copied selection available and are reported through `self.error`.
    fn cut(&mut self, ctx: &egui::Context) {
        if self.pending() {
            self.error = Some("Apply or reset the property fields before cutting.".into());
            return;
        }
        if self.selection.is_empty() {
            return;
        }
        self.copy(ctx);
        self.delete();
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
    /// Preview the library's generated reference using default physical silkscreen settings.
    /// Authoring revisions invalidate the paths, while zoom and pan only reproject them.
    fn reference_preview(&mut self, item: &Item) -> Option<std::sync::Arc<[Vec<Point>]>> {
        item.reference_font.as_ref()?;
        let revision = self.session.revision();
        if let Some((cached, paths)) = &self.reference_preview
            && *cached == revision
        {
            return Some(paths.clone());
        }
        let definition = item.footprint().ok()?;
        let paths: std::sync::Arc<[_]> =
            definition.reference_paths("REF", Default::default(), pcbcraft_engine::board::DEFAULT_REFERENCE_OFFSET).into();
        self.reference_preview = Some((revision, paths.clone()));
        Some(paths)
    }

    /// Navigate and edit the focused pane through revision-checked transactions. Directional
    /// boxes, modifier selection, clipboard placement and repeated drawing share stable IDs.
    pub(super) fn canvas(&mut self, ui: &mut Ui, area: Rect, covered: [Rect; 2], navigation: Navigation, active: bool) {
        use pcbcraft_engine::library_graphics::{Fill, Geometry, Graphic};
        let item = self.session.snapshot();
        let reference = self.reference_preview(&item);
        let t = Tokens::get();
        let response = ui.interact(area, ui.scope_id().with("library_canvas"), egui::Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::GenericContainer, ui.is_enabled(), "Library canvas"));
        if !ui.input(|i| i.focused) {
            self.tool = Tool::Select;
        }
        let selecting_at_start = matches!(self.tool, Tool::Select);
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
        let extents = item.objects.values().flat_map(points).chain(reference.iter().flat_map(|paths| paths.iter().flatten().copied()));
        self.camera.navigate_definition(&response, area, extents, over, navigation);
        let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(area);
        painter.rect_filled(area, 0, if item.kind == Kind::Symbol { t.canvas } else { t.board_canvas });
        let origin = self.camera.screen(Point { x_nm: 0, y_nm: 0 }, area);
        grid::paint(&painter, area, origin, self.camera.snap_step(), authoring::GRID_OPACITY);
        let path_cancelled = matches!(self.tool, Tool::EditPath { revision, .. } if revision != self.session.revision()
            || !active || !ui.is_enabled()
            || ui.input(|input| input.key_pressed(egui::Key::Escape) || input.pointer.secondary_pressed()));
        if path_cancelled {
            self.tool = Tool::Select;
        }
        self.show_vertex_menu(&response, &item, area, over && !path_cancelled, active && ui.is_enabled());
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
        let show_courtyard = self.show_courtyard;
        let visible = |p: &Primitive| show_courtyard || !pcbcraft_engine::authoring::courtyard::is_courtyard_primitive(p);
        if over && !path_cancelled && response.drag_started() && selecting_at_start && matches!(self.tool, Tool::Select) && !self.pending() {
            let start = ui.input(|i| i.pointer.press_origin()).unwrap_or_else(|| pointer.unwrap_or(area.center()));
            let found = pick_visible(&item, &self.camera, area, start, show_courtyard);
            let canvas = PathCanvas { camera: &self.camera, rect: area };
            let path_drag = self
                .selected
                .filter(|id| found.is_none_or(|found| found == *id))
                .filter(|_| response.drag_started_by(egui::PointerButton::Primary) && !ui.input(|i| i.modifiers.shift || i.modifiers.command))
                .and_then(|id| {
                    let path = DefinitionPath::of(item.objects.get(&id)?)?.outline();
                    let handle = canvas.handle(start, path);
                    (handle.is_some() || canvas.hit_body(start, path)).then(|| (id, PathDrag::new(canvas, path, start, handle)))
                });
            if show_courtyard
                && let Some((object, pts, _)) = self.selected_courtyard(&item)
                && let Some(vertex) = courtyard::pick_vertex(pts, &self.camera, area, start)
                && let Some(original) = item.objects.get(&object).cloned()
            {
                self.selected_vertex = Some(vertex);
                self.tool = Tool::DragCourtyardVertex { object, vertex, original, revision: self.session.revision() };
            } else if let Some((object, drag)) = path_drag {
                self.tool = Tool::EditPath { object, drag, revision: self.session.revision() };
            } else if let Some(id) = found {
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
        // Keyboard commands follow the same rule as the tool reset above: a batch from an
        // unfocused viewport carries no key intent the canvas may act on, so no handler —
        // including the destructive M/E/Delete/Enter keys and Escape — may read key_pressed.
        let keyboard_active = active && ui.is_enabled() && !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.focused);
        if keyboard_active && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.tool = Tool::Select;
        }
        let mut preview = Vec::new();
        let mut drag_invalid: Option<Point> = None;
        let mut path_preview = None;
        let mut path_alignment = None;
        if let Some(point) = pointer.filter(|_| over && !path_cancelled) {
            let canvas = PathCanvas { camera: &self.camera, rect: area };
            let bypass = ui.input(|input| input.modifiers.alt);
            let at = if let Tool::Polyline { points, .. } = &self.tool {
                let closure = if points.len() >= 3 { canvas.nearby_vertex(point, points.first().copied()) } else { None };
                let snap = canvas.snap(point, points.last().copied(), closure, bypass);
                path_alignment = Some(snap);
                snap.point
            } else {
                self.camera.snap(point, area)
            };
            if let Tool::EditPath { object, drag, .. } = &mut self.tool {
                let external = item
                    .objects
                    .iter()
                    .filter(|(id, _)| **id != *object)
                    .filter_map(|(_, primitive)| DefinitionPath::of(primitive))
                    .flat_map(|path| path.outline().points.iter().copied());
                drag.update(canvas, point, external, bypass);
                path_alignment = drag.alignment;
            }
            preview = match &self.tool {
                Tool::EditPath { object, drag, .. } => {
                    path_preview = Some(*object);
                    item.objects.get(object).and_then(DefinitionPath::of).map(|path| path.replace(drag.points.clone())).into_iter().collect()
                }
                Tool::Move { original, .. } => original
                    .translated(Point { x_nm: at.x_nm - original.anchor().x_nm, y_nm: at.y_nm - original.anchor().y_nm })
                    .into_iter()
                    .collect(),
                Tool::MoveGroup { originals, anchor, .. } | Tool::Paste { primitives: originals, anchor, .. } => {
                    originals.iter().filter_map(|p| p.translated(Point { x_nm: at.x_nm - anchor.x_nm, y_nm: at.y_nm - anchor.y_nm }).ok()).collect()
                }
                Tool::DragCourtyardVertex { vertex, original, .. } => {
                    match pcbcraft_engine::authoring::courtyard::with_moved_vertex(original, *vertex, at) {
                        Ok(primitive) => vec![primitive],
                        Err(_) => {
                            drag_invalid = Some(at);
                            Vec::new()
                        }
                    }
                }
                Tool::Draw { kind: Place::Courtyard, .. } => Vec::new(),
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
                    // A release that ends a drag with no live tool (Escape cancelled the move,
                    // box or vertex drag, or pending fields kept one from starting) is not a
                    // click: it never selects, toggles or inserts at the release point.
                    Tool::Select if !response.clicked() => Ok(()),
                    Tool::Select => {
                        // Extend/toggle follows the release event's own modifiers — a
                        // modifier released or arriving in the same batch cannot swap
                        // the click's meaning after the fact.
                        let release_modifiers = ui.input(|i| {
                            i.events
                                .iter()
                                .rev()
                                .find_map(|event| match event {
                                    egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, modifiers, .. } => {
                                        Some(*modifiers)
                                    }
                                    _ => None,
                                })
                                .unwrap_or(i.modifiers)
                        });
                        let modify = release_modifiers.shift || release_modifiers.command;
                        if show_courtyard
                            && !modify
                            && let Some((_, pts, _)) = self.selected_courtyard(&item)
                            && let Some(vertex) = courtyard::pick_vertex(pts, &self.camera, area, point)
                        {
                            if self.pending() {
                                Err("Apply or reset the property fields before switching vertices.".into())
                            } else {
                                self.selected_vertex = Some(vertex);
                                Ok(())
                            }
                        } else if show_courtyard
                            && !modify
                            && !self.pending()
                            && let Some((object, pts, _)) = self.selected_courtyard(&item)
                            && pts.len() < pcbcraft_engine::authoring::courtyard::MAX_VERTICES
                            && let Some(edge) = courtyard::pick_edge_midpoint(pts, &self.camera, area, point)
                            && let Some(primitive) = item.objects.get(&object)
                        {
                            let inserted = pcbcraft_engine::authoring::courtyard::edge_midpoint(pts, edge)
                                .ok_or_else(|| "Unable to compute edge midpoint.".to_owned())
                                .and_then(|mid| pcbcraft_engine::authoring::courtyard::with_inserted_vertex(primitive, edge, mid))
                                .and_then(|candidate| self.execute(self.session.revision(), Action::Replace { object, primitive: candidate }));
                            if inserted.is_ok() {
                                self.selected_vertex = Some(edge + 1);
                            }
                            inserted
                        } else {
                            let found = pick_visible(&item, &self.camera, area, point, show_courtyard);
                            let mut ids = if modify { self.selection.clone() } else { Default::default() };
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
                    }
                    Tool::DragCourtyardVertex { object, vertex, original, revision } => {
                        let result = pcbcraft_engine::authoring::courtyard::with_moved_vertex(&original, vertex, at)
                            .and_then(|primitive| self.execute(revision, Action::Replace { object, primitive }));
                        if result.is_ok() {
                            self.selected_vertex = Some(vertex);
                            self.tool = Tool::Select;
                        }
                        result
                    }
                    Tool::Move { object, revision, .. } => preview
                        .first()
                        .cloned()
                        .ok_or("No move preview.".into())
                        .and_then(|primitive| self.execute(revision, Action::Replace { object, primitive })),
                    Tool::EditPath { object, revision, .. } => {
                        let result = preview
                            .first()
                            .cloned()
                            .ok_or("No outline preview.".into())
                            .and_then(|primitive| self.execute(revision, Action::Replace { object, primitive }));
                        self.tool = Tool::Select;
                        result
                    }
                    Tool::VertexMenu { .. } => Ok(()),
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
                                    visible(p) && {
                                        let b = bounds(p, &self.camera, area);
                                        if crossing { rect.intersects(b) } else { rect.contains_rect(b) }
                                    }
                                })
                                .map(|(id, _)| *id),
                        );
                        let result = self.select_many(initial);
                        self.tool = Tool::Select;
                        result
                    }
                    Tool::Draw { kind: Place::Courtyard, revision, .. } => {
                        self.tool = Tool::Courtyard { points: vec![at], revision };
                        Ok(())
                    }
                    Tool::Courtyard { mut points, revision } => {
                        let closing = points.len() >= pcbcraft_engine::authoring::courtyard::MIN_VERTICES
                            && (points.first() == Some(&at) || response.double_clicked());
                        if !closing && points.last() != Some(&at) {
                            points.push(at);
                        }
                        if closing || points.len() >= pcbcraft_engine::authoring::courtyard::MAX_VERTICES {
                            match pcbcraft_engine::authoring::courtyard::make_courtyard_polygon(points, self.drawing_stroke_nm) {
                                Ok(primitive) => {
                                    let result = self.execute(revision, Action::Insert { primitive });
                                    if result.is_ok() {
                                        self.show_courtyard = true;
                                        self.tool = Tool::Select;
                                    }
                                    result
                                }
                                Err(error) if revision != self.session.revision() => Err(error),
                                Err(error) => {
                                    self.error = Some(error);
                                    Ok(())
                                }
                            }
                        } else {
                            self.tool = Tool::Courtyard { points, revision };
                            Ok(())
                        }
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
        } else if response.drag_stopped() || matches!(self.tool, Tool::EditPath { .. }) && !ui.input(|i| i.pointer.primary_down()) {
            self.tool = Tool::Select;
        }
        if keyboard_active && over {
            if ui.input(|i| {
                i.focused
                    && i.events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Key { key: egui::Key::A, pressed: true, modifiers, .. } if modifiers.command))
            }) {
                let _ = self.select_many(item.objects.iter().filter(|(_, p)| visible(p)).map(|(id, _)| *id).collect());
            }
            // Command-only C/X/V arrive as normalized clipboard events on desktop; a raw
            // key press only appears when another modifier bypasses that translation.
            // egui-winit emits Paste("") when the OS clipboard read is empty and
            // PasteImage when it holds only an image — both mean "paste intent" here.
            // Paste and duplicate arm the same preview and resolve in event order, only
            // while focused: a later Escape in the same batch cancels the chord, and a
            // focus-loss batch must not arm a preview the reset just cleared.
            let copied = ui.input(|i| {
                i.focused
                    && i.events.iter().any(|event| match event {
                        egui::Event::Copy => true,
                        egui::Event::Key { key: egui::Key::C, pressed: true, modifiers, .. } => modifiers.command,
                        _ => false,
                    })
            });
            let cut = ui.input(|i| {
                i.focused
                    && i.events.iter().any(|event| match event {
                        egui::Event::Cut => true,
                        egui::Event::Key { key: egui::Key::X, pressed: true, modifiers, .. } => modifiers.command,
                        _ => false,
                    })
            });
            let preview = ui.input(|i| {
                if !i.focused {
                    return None;
                }
                i.events.iter().fold(None, |preview, event| match event {
                    egui::Event::Paste(_) | egui::Event::PasteImage(_) => Some(Preview::Paste),
                    egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => None,
                    egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. } if modifiers.command => Some(Preview::Paste),
                    egui::Event::Key { key: egui::Key::D, pressed: true, modifiers, .. } if modifiers.command => Some(Preview::Duplicate),
                    _ => preview,
                })
            });
            if copied {
                self.copy(ui.ctx());
            }
            if cut {
                self.cut(ui.ctx());
            }
            match preview {
                Some(Preview::Paste) => self.paste(ui.ctx()),
                Some(Preview::Duplicate) => self.duplicate(),
                None => {}
            }
            // Tool letters and R's direction read the pressed event's own modifiers,
            // like the command chords above: a command released before batch end
            // cannot leak a chord into a tool start, and a shift released early
            // cannot flip the rotation direction. Tool starts also resolve against a
            // same-batch Escape in event order: a letter pressed before the Escape
            // stays cancelled, and one pressed after it still arms.
            let tool_key = |key: egui::Key, modifiers_ok: fn(&egui::Modifiers) -> bool| {
                ui.input(|i| {
                    if !i.focused {
                        return false;
                    }
                    i.events.iter().fold(false, |armed, event| match event {
                        egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => false,
                        egui::Event::Key { key: letter, pressed: true, modifiers, .. } if *letter == key && modifiers_ok(modifiers) => true,
                        _ => armed,
                    })
                })
            };
            if tool_key(egui::Key::P, |modifiers| !modifiers.command) {
                self.start_tool(if item.kind == Kind::Symbol { Place::Pin } else { Place::Pad });
            }
            if tool_key(egui::Key::L, |modifiers| !modifiers.command) {
                self.start_tool(Place::Line);
            }
            if tool_key(egui::Key::T, |modifiers| !modifiers.command) {
                self.start_tool(Place::Text);
            }
            if item.kind == Kind::Footprint && tool_key(egui::Key::C, |modifiers| modifiers.is_none()) {
                self.start_tool(Place::Courtyard);
                if matches!(self.tool, Tool::Draw { kind: Place::Courtyard, .. }) {
                    self.set_show_courtyard(true);
                }
            }
            if tool_key(egui::Key::M, |_| true) {
                self.begin_move();
            }
            if let Some(shifted) = ui.input(|i| {
                if !i.focused {
                    return None;
                }
                i.events.iter().find_map(|event| match event {
                    egui::Event::Key { key: egui::Key::R, pressed: true, modifiers, .. } => Some(modifiers.shift),
                    _ => None,
                })
            }) {
                self.rotate_by(if shifted { 1 } else { -1 });
            }
            if ui.input(|i| i.key_pressed(egui::Key::E)) && self.selection.len() == 1 {
                self.details_requested = true;
            }
            // Completing or cancelling another tool in this pass must not redirect its key
            // events to the saved selection.
            if selecting_at_start
                && matches!(self.tool, Tool::Select)
                && ui.input(|i| {
                    i.focused
                    && i.events.iter().any(
                        |event| matches!(event, egui::Event::Key { key: egui::Key::Backspace, pressed: true, modifiers, .. } if modifiers.is_none()),
                    )
                })
            {
                // A selected vertex remains the target even when deletion is unavailable.
                // Read the outline after this frame's edits: a midpoint click earlier in the
                // same frame has already inserted the vertex that is now selected.
                let current = self.session.snapshot();
                if let Some((object, pts, _)) = self.selected_courtyard(&current)
                    && let Some(vertex) = self.selected_vertex
                    && let Some(primitive) = current.objects.get(&object)
                {
                    if !self.show_courtyard {
                        self.error = Some("Show the Courtyard layer before removing a vertex.".into());
                    } else if self.pending() {
                        self.error = Some("Apply or reset the property fields before removing a vertex.".into());
                    } else {
                        match pcbcraft_engine::authoring::courtyard::with_removed_vertex(primitive, vertex)
                            .and_then(|candidate| self.execute(self.session.revision(), Action::Replace { object, primitive: candidate }))
                        {
                            Ok(()) => {
                                let next_len = pts.len().saturating_sub(1);
                                self.selected_vertex = (next_len > 0).then(|| vertex.min(next_len - 1));
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                } else {
                    self.delete();
                }
            }
            if matches!(self.tool, Tool::Select) && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.delete();
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                match self.tool.clone() {
                    Tool::Polyline { points, revision } => {
                        if let Err(error) = self.execute(
                            revision,
                            Action::Insert { primitive: graphic_object(points, false, self.drawing_purpose, self.drawing_stroke_nm) },
                        ) {
                            self.error = Some(error);
                        }
                    }
                    Tool::Courtyard { points, revision } => {
                        match pcbcraft_engine::authoring::courtyard::make_courtyard_polygon(points, self.drawing_stroke_nm)
                            .and_then(|primitive| self.execute(revision, Action::Insert { primitive }))
                        {
                            Ok(()) => {
                                self.show_courtyard = true;
                                self.tool = Tool::Select;
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                    _ => {}
                }
            }
        }
        // Body fills stay behind terminal labels regardless of insertion order.
        // Courtyard primitives render on their own layer pass above body/copper when enabled.
        let is_courtyard = |p: &&Primitive| pcbcraft_engine::authoring::courtyard::is_courtyard_primitive(p);
        let filled = |p: &&Primitive| matches!(p, Primitive::Graphic { graphic } if graphic.fill != Fill::None);
        let objects = item
            .objects
            .iter()
            .filter(|(_, p)| !is_courtyard(p) && filled(p))
            .chain(item.objects.iter().filter(|(_, p)| !is_courtyard(p) && !filled(p)));
        let is_moving = |id: &u64| {
            matches!(&self.tool, Tool::Move { object, .. } | Tool::DragCourtyardVertex { object, .. } if object == id)
                || matches!(&self.tool, Tool::MoveGroup { objects, .. } if objects.contains(id))
        };
        for (id, p) in objects {
            if (path_preview != Some(*id) && !is_moving(id)) || preview.is_empty() {
                paint(&painter, p, &self.camera, area, self.selection.contains(id));
                if self.selected == Some(*id)
                    && let Some(path) = DefinitionPath::of(p)
                {
                    board::drawing_vertices(&painter, path.outline().points.iter().map(|point| self.camera.screen(*point, area)));
                }
            }
        }
        if self.show_courtyard {
            for (id, p) in item.objects.iter().filter(|(_, p)| is_courtyard(p)) {
                if !is_moving(id) || preview.is_empty() {
                    paint(&painter, p, &self.camera, area, self.selection.contains(id));
                }
            }
        }
        for p in &preview {
            paint(&painter, p, &self.camera, area, true);
            if path_preview.is_some()
                && let Some(path) = DefinitionPath::of(p)
            {
                board::drawing_vertices(&painter, path.outline().points.iter().map(|point| self.camera.screen(*point, area)));
            }
        }
        if let Some(snap) = path_alignment {
            (PathCanvas { camera: &self.camera, rect: area }).paint_guides(&painter, snap);
        }
        if self.show_courtyard
            && let Tool::Courtyard { points, .. } = &self.tool
            && let Some(point) = pointer.filter(|_| over)
        {
            courtyard::paint_draw_preview(&painter, points, self.camera.snap(point, area), &self.camera, area);
        }
        if self.show_courtyard
            && matches!(self.tool, Tool::Select | Tool::DragCourtyardVertex { .. })
            && let Some((_, pts, _)) = self.selected_courtyard(&item)
        {
            let hovered_vertex = pointer.filter(|_| over).and_then(|p| courtyard::pick_vertex(pts, &self.camera, area, p));
            let hovered_edge = if hovered_vertex.is_none() && pts.len() < pcbcraft_engine::authoring::courtyard::MAX_VERTICES {
                pointer.filter(|_| over).and_then(|p| courtyard::pick_edge_midpoint(pts, &self.camera, area, p))
            } else {
                None
            };
            courtyard::paint_handles(
                &painter,
                pts,
                self.selected_vertex,
                hovered_vertex,
                hovered_edge,
                &self.camera,
                area,
                !self.pending() && pts.len() < pcbcraft_engine::authoring::courtyard::MAX_VERTICES,
            );
        }
        if let Some(bad) = drag_invalid {
            painter.circle_stroke(self.camera.screen(bad, area), courtyard::VERTEX_HANDLE_RADIUS + 3.0, egui::Stroke::new(2.0, t.error));
        }
        if let Some(paths) = &reference {
            let width = pcbcraft_engine::board::silkscreen::Settings::default().stroke_width_nm as f32 / pcbcraft_engine::schematic::GRID_NM as f32
                * self.camera.unit_step();
            let stroke = egui::Stroke::new(width, pcbcraft_chrome::board::graphic_color(pcbcraft_chrome::board::GraphicStyle::Silkscreen));
            for path in paths.iter() {
                painter.add(egui::Shape::line(path.iter().map(|point| self.camera.screen(*point, area)).collect(), stroke));
            }
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
                    visible(p) && {
                        let b = bounds(p, &self.camera, area);
                        if crossing { rect.intersects(b) } else { rect.contains_rect(b) }
                    }
                })
                .count();
            pcbcraft_chrome::selection::marquee(&painter, rect, crossing, count);
        }
        authoring::origin(&painter, origin);
    }
}
/// Allocate the next unused electrical number; object allocation remains inside the engine.
fn primitive(item: &Item, kind: Place, start: Point, end: Point) -> Result<Primitive, String> {
    if kind == Place::NonPlatedHole {
        return Ok(Primitive::NonPlatedHole { hole: foot::CircularNpth { center: end, diameter_nm: 1_000_000 } });
    }
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
        Place::Pin => Primitive::Pin {
            number,
            name: None,
            electrical_type: ElectricalType::Passive,
            alternates: Vec::new(),
            at: end,
            appearance: Some(Default::default()),
        },
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
                plated_slot: None,
            },
        },
        Place::Arc | Place::Polyline | Place::NonPlatedHole | Place::Courtyard => {
            return Err("Choose drawing geometry through its parameterized tool.".into());
        }
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
        Primitive::NonPlatedHole { hole } => board::non_plated_hole(painter, at(hole.center), hole.diameter_nm as f32 * scale / 2., selected),
        Primitive::Pad { pad } => board::footprint(
            painter,
            &[board::PadDrawing {
                color: Some(if pad.copper == CopperBinding::AllCopper { Tokens::get().board_through } else { board::copper_color(Some(true)) }),
                number: pad.number.clone(),
                center: at(pad.center),
                copper: Some(if pad.plated_slot.is_some() {
                    pad.local_corners().map(at)
                } else {
                    corners(pad.center, pad.width_nm, pad.height_nm, pad.rotation)
                }),
                contour: match pad.shape {
                    PadShape::Rectangle => board::PadContour::Rectangle,
                    PadShape::Circle => board::PadContour::Circle,
                    PadShape::Oval => board::PadContour::Oval,
                    PadShape::RoundedRectangle => board::PadContour::RoundedRectangle { radius: pad.corner_radius_nm as f32 * scale },
                },
                drill_radius: pad.drill_nm.map(|r| r as f32 * scale / 2.0),
                plated_slot: pad
                    .plated_slot
                    .and_then(|slot| slot.resolve(pad.center, 0, false))
                    .map(|slot| (slot.cap_centres().map(at), slot.width_nm() as f32 * scale / 2.0)),
                through_hole: pad.copper == CopperBinding::AllCopper,
                net: None,
            }],
            &[],
            "",
            at(pad.center),
            board::copper_color(Some(true)),
            selected,
            false,
            false,
            egui::Vec2::ZERO,
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
                reference: false,
            }],
            "",
            at(graphic.center),
            board::copper_color(Some(true)),
            selected,
            false,
            false,
            egui::Vec2::ZERO,
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
                reference: false,
            }],
            "",
            at(primitive.anchor()),
            board::copper_color(Some(true)),
            selected,
            false,
            false,
            egui::Vec2::ZERO,
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
        Primitive::NonPlatedHole { hole } => pointer.distance(at(hole.center)) <= (hole.diameter_nm as f32 * scale / 2.).max(authoring::PICK_PADDING),
        Primitive::Outline { graphic } => rectangle(graphic.center, graphic.width_nm, graphic.height_nm, graphic.rotation, false),
        Primitive::Silkscreen { path } => {
            path.points.windows(2).any(|pair| near(at(pair[0]), at(pair[1])))
                || (path.closed && path.points.first().zip(path.points.last()).is_some_and(|(a, b)| near(at(*a), at(*b))))
        }
    }
}

/// Electrical objects win overlaps with body graphics so labels and lands remain directly editable.
/// Hidden courtyard primitives are skipped so an invisible layer never intercepts clicks.
fn pick_visible(item: &Item, camera: &crate::canvas_camera::Camera, area: Rect, at: egui::Pos2, show_courtyard: bool) -> Option<u64> {
    let visible = |p: &&Primitive| show_courtyard || !pcbcraft_engine::authoring::courtyard::is_courtyard_primitive(p);
    item.objects
        .iter()
        .rev()
        .filter(|(_, p)| visible(p) && matches!(p, Primitive::Pin { .. } | Primitive::Pad { .. }))
        .chain(item.objects.iter().rev().filter(|(_, p)| visible(p) && !matches!(p, Primitive::Pin { .. } | Primitive::Pad { .. })))
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

    /// Shared Select behaviour in both library editors, shown in the Symbol editor: releasing
    /// over another pin after Escape cancelled a group move keeps the selection and revision,
    /// while a real click there selects it; unmodified Backspace deletes the selection in one
    /// undoable edit and Command+Backspace does not.
    #[test]
    fn symbol_release_after_cancelled_move_keeps_selection_and_backspace_deletes() {
        use egui_kittest::{Harness, kittest::Queryable};
        /// Press or release the primary button at `pos`. Each press first idles past the
        /// double-click interval in logical time, so consecutive clicks never pair up into a
        /// double-click (which would open the item properties dialog).
        fn press(h: &mut Harness<'_, crate::PcbApp>, pos: egui::Pos2, pressed: bool) {
            if pressed {
                h.run_steps(30);
            }
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run();
        }
        let mut app = crate::PcbApp::default();
        app.new_library_item(Kind::Symbol);
        let id = app.library_editors.active.unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut crate::PcbApp| app.frame(ui), app);
        h.run();
        let area = h.get_by_label("Library canvas").rect();
        let (pin_in, pin_out) = {
            let doc = &h.state().library_editors.documents[&id];
            let item = doc.session.snapshot();
            let at = |object: u64| match item.objects.get(&object) {
                Some(Primitive::Pin { at, .. }) => doc.camera.screen(*at, area),
                other => panic!("expected pin {object}, found {other:?}"),
            };
            (at(1), at(3))
        };
        for pos in [pin_in, pin_out] {
            assert!(area.contains(pos));
        }
        h.hover_at(pin_in);
        h.run();
        press(&mut h, pin_in, true);
        press(&mut h, pin_in, false);
        assert_eq!(h.state().library_editors.documents[&id].selection.iter().copied().collect::<Vec<_>>(), vec![1]);
        let rev = h.state().library_editors.documents[&id].session.revision();

        // Drag pin 1 toward pin 2, cancel with Escape, release over pin 2.
        press(&mut h, pin_in, true);
        h.hover_at(pin_out);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, Tool::MoveGroup { .. }));
        h.key_press(egui::Key::Escape);
        h.run();
        press(&mut h, pin_out, false);
        let doc = &h.state().library_editors.documents[&id];
        assert!(matches!(doc.tool, Tool::Select));
        assert_eq!(doc.selection.iter().copied().collect::<Vec<_>>(), vec![1]);
        assert_eq!(doc.session.revision(), rev);

        // Positive control: a real click at the same point selects pin 2.
        press(&mut h, pin_out, true);
        press(&mut h, pin_out, false);
        assert_eq!(h.state().library_editors.documents[&id].selection.iter().copied().collect::<Vec<_>>(), vec![3]);
        assert!(matches!(h.state().dialog, Dialog::None));

        // Command+Backspace leaves the pin; plain Backspace deletes it as one undoable edit.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Backspace);
        h.run();
        assert!(h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&3));
        h.key_press(egui::Key::Backspace);
        h.run();
        assert!(!h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&3));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev + 1);
        crate::tests::click(&mut h, "Undo Library Edit");
        assert!(h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&3));
    }

    /// Hidden courtyard outlines never intercept clicks; visible outlines expose vertex and
    /// midpoint handles for direct dragging and insertion.
    #[test]
    fn courtyard_vertex_pick_and_hidden_layer_picking() {
        let mut editors = super::super::Editors::default();
        let id = editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let doc = editors.documents.get_mut(&id).unwrap();
        // Zoom in so the 0.25 mm courtyard clearance sits well outside the pad pick padding.
        doc.camera.set_zoom(20.0);
        doc.execute(
            0,
            Action::GenerateCourtyard {
                clearance_nm: pcbcraft_engine::authoring::courtyard::DEFAULT_CLEARANCE_NM,
                source: pcbcraft_engine::authoring::courtyard::CourtyardSource::PadsAndBody,
                stroke_nm: pcbcraft_engine::authoring::courtyard::DEFAULT_STROKE_NM,
                replace_existing: true,
            },
        )
        .unwrap();
        let item = doc.session.snapshot();
        let (cy_id, pts, _) = doc.selected_courtyard(&item).unwrap();
        let area = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
        let v0_screen = doc.camera.screen(pts[0], area);
        let mid0 = pcbcraft_engine::authoring::courtyard::edge_midpoint(pts, 0).unwrap();
        let mid0_screen = doc.camera.screen(mid0, area);

        assert_eq!(pick_visible(&item, &doc.camera, area, v0_screen, false), None);
        assert_eq!(pick_visible(&item, &doc.camera, area, v0_screen, true), Some(cy_id));
        assert_eq!(courtyard::pick_vertex(pts, &doc.camera, area, v0_screen), Some(0));
        assert_eq!(courtyard::pick_edge_midpoint(pts, &doc.camera, area, mid0_screen), Some(0));
    }

    /// Dragging a courtyard vertex snaps to the active footprint grid, commits a single
    /// undoable edit, and rejects moves that would make the outline self-intersect.
    #[test]
    fn courtyard_vertex_drag_snaps_to_grid_and_commits_one_edit() {
        let mut editors = super::super::Editors::default();
        let id = editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
        let doc = editors.documents.get_mut(&id).unwrap();
        doc.camera.set_snap_nm(250_000);
        doc.execute(
            0,
            Action::GenerateCourtyard {
                clearance_nm: pcbcraft_engine::authoring::courtyard::DEFAULT_CLEARANCE_NM,
                source: pcbcraft_engine::authoring::courtyard::CourtyardSource::PadsAndBody,
                stroke_nm: pcbcraft_engine::authoring::courtyard::DEFAULT_STROKE_NM,
                replace_existing: true,
            },
        )
        .unwrap();
        let area = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
        let item = doc.session.snapshot();
        let (cy_id, pts, _) = doc.selected_courtyard(&item).unwrap();
        let original = item.objects[&cy_id].clone();
        let target_screen = doc.camera.screen(Point { x_nm: -1_480_000, y_nm: -980_000 }, area);
        let snapped = doc.camera.snap(target_screen, area);
        assert_eq!(snapped.x_nm % 250_000, 0);
        assert_eq!(snapped.y_nm % 250_000, 0);

        // Moving vertex 0 across the opposite edge creates a self-intersection and is refused.
        let crossed = Point { x_nm: (pts[2].x_nm + pts[3].x_nm) / 2, y_nm: pts[2].y_nm + 1_000_000 };
        assert!(pcbcraft_engine::authoring::courtyard::with_moved_vertex(&original, 0, crossed).is_err());

        let moved = pcbcraft_engine::authoring::courtyard::with_moved_vertex(&original, 0, snapped).unwrap();
        let rev = doc.session.revision();
        doc.execute(rev, Action::Replace { object: cy_id, primitive: moved }).unwrap();
        let updated = doc.session.snapshot();
        let (next_pts, _) = pcbcraft_engine::authoring::courtyard::courtyard_polygon(&updated.objects[&cy_id]).unwrap();
        assert_eq!(next_pts[0], snapped);
    }

    /// Plain Backspace and Delete only remove the selected primitive in Select mode; active
    /// drawing tools (including Courtyard) keep the selection intact.
    #[test]
    fn backspace_and_delete_only_remove_selection_in_select_tool() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = crate::PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut crate::PcbApp| app.frame(ui), app);
        h.run();
        let object = *h.state().library_editors.documents[&id].session.snapshot().objects.keys().next().unwrap();
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select(Some(object)).unwrap();
        h.run();
        let area = h.get_by_label("Library canvas").rect();
        h.hover_at(area.center());
        h.run();
        let rev = h.state().library_editors.documents[&id].session.revision();

        // Courtyard drawing tool keeps the selected primitive on Backspace and Delete.
        h.key_press(egui::Key::C);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, Tool::Draw { kind: Place::Courtyard, .. }));
        assert!(h.state().library_editors.documents[&id].selection.contains(&object));
        h.key_press(egui::Key::Backspace);
        h.run();
        assert!(h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&object));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);
        h.key_press(egui::Key::Delete);
        h.run();
        assert!(h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&object));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);

        // Other drawing tools (e.g. Line) also keep the selected primitive on Backspace and Delete.
        h.key_press(egui::Key::L);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, Tool::Draw { kind: Place::Line, .. }));
        assert!(h.state().library_editors.documents[&id].selection.contains(&object));
        h.key_press(egui::Key::Backspace);
        h.run();
        assert!(h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&object));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);
        h.key_press(egui::Key::Delete);
        h.run();
        assert!(h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&object));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);

        // Returning to Select mode restores Delete and plain Backspace deletion.
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, Tool::Select));
        h.key_press(egui::Key::Delete);
        h.run();
        assert!(!h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&object));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev + 1);

        crate::tests::click(&mut h, "Undo Library Edit");
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select(Some(object)).unwrap();
        h.run();
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::Backspace);
        h.run();
        assert!(!h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&object));
    }

    /// Closing a self-intersecting courtyard draft by clicking the first vertex reports the
    /// validation error while keeping the drawing tool and editable points; a valid mouse close
    /// still commits in one edit and exits to Select, and a stale captured revision discards the
    /// draft so a fresh draft is required.
    #[test]
    fn courtyard_mouse_close_keeps_draft_on_geometry_error_and_discards_stale_revision() {
        use egui_kittest::{Harness, kittest::Queryable};
        fn click(h: &mut Harness<'_, crate::PcbApp>, pos: egui::Pos2) {
            h.hover_at(pos);
            h.run();
            h.run_steps(30);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run();
            }
        }

        let mut app = crate::PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut crate::PcbApp| app.frame(ui), app);
        h.run();
        if let Some(doc) = h.state_mut().library_editors.documents.get_mut(&id) {
            let zoom = doc.camera.zoom();
            doc.camera.set_zoom(zoom / 4.0);
        }
        h.run();
        let area = h.get_by_label("Library canvas").rect();
        let p0 = Point { x_nm: -1_000_000, y_nm: -1_000_000 };
        let p1 = Point { x_nm: 1_000_000, y_nm: 1_000_000 };
        let p2 = Point { x_nm: 1_000_000, y_nm: -1_000_000 };
        let p3 = Point { x_nm: -1_000_000, y_nm: 1_000_000 };
        let (s0, s1, s2, s3) = {
            let doc = &h.state().library_editors.documents[&id];
            (doc.camera.screen(p0, area), doc.camera.screen(p1, area), doc.camera.screen(p2, area), doc.camera.screen(p3, area))
        };
        for pos in [s0, s1, s2, s3] {
            assert!(area.contains(pos));
        }

        let rev0 = h.state().library_editors.documents[&id].session.revision();
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        click(&mut h, s0);
        click(&mut h, s1);
        click(&mut h, s2);
        click(&mut h, s3);
        assert!(matches!(
            &h.state().library_editors.documents[&id].tool,
            Tool::Courtyard { points, revision } if points.as_slice() == [p0, p1, p2, p3] && *revision == rev0
        ));

        // Clicking the first vertex on a self-intersecting draft reports an error but keeps the
        // drawing tool and editable points so the user does not lose the outline.
        click(&mut h, s0);
        {
            let doc = &h.state().library_editors.documents[&id];
            assert!(doc.error.is_some());
            assert!(matches!(
                &doc.tool,
                Tool::Courtyard { points, revision } if points.as_slice() == [p0, p1, p2, p3] && *revision == rev0
            ));
            assert_eq!(doc.session.revision(), rev0);
        }

        // Cancel the invalid draft and verify a valid mouse close still commits in one edit.
        h.key_press(egui::Key::Escape);
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        click(&mut h, s0);
        click(&mut h, s2);
        click(&mut h, s1);
        click(&mut h, s0);
        let cy_id = {
            let doc = &h.state().library_editors.documents[&id];
            assert_eq!(doc.error, None);
            assert!(matches!(doc.tool, Tool::Select));
            assert!(doc.show_courtyard);
            assert_eq!(doc.session.revision(), rev0 + 1);
            let cy_id = doc.selected.unwrap();
            assert_eq!(pcbcraft_engine::authoring::courtyard::courtyard_polygon(&doc.session.snapshot().objects[&cy_id]).unwrap().0.len(), 3);
            cy_id
        };

        // A stale captured revision still discards the draft and returns to Select on close.
        let rev1 = h.state().library_editors.documents[&id].session.revision();
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        click(&mut h, s0);
        click(&mut h, s2);
        click(&mut h, s1);
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().session.execute(rev1, Action::Delete { objects: vec![cy_id] }).unwrap();
        let rev_stale = h.state().library_editors.documents[&id].session.revision();
        click(&mut h, s0);
        {
            let doc = &h.state().library_editors.documents[&id];
            assert!(doc.error.is_some());
            assert!(matches!(doc.tool, Tool::Select));
            assert_eq!(doc.session.revision(), rev_stale);
        }
    }

    /// Escape and pointer release in the same frame cancel a courtyard vertex drag without
    /// committing Action::Replace, while a release without Escape still commits the move.
    #[test]
    fn courtyard_vertex_drag_same_frame_escape_and_release_does_not_commit() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = crate::PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        {
            let doc = app.library_editors.documents.get_mut(&id).unwrap();
            doc.camera.set_zoom(20.0);
            doc.camera.set_snap_nm(250_000);
            doc.execute(
                0,
                Action::GenerateCourtyard {
                    clearance_nm: pcbcraft_engine::authoring::courtyard::DEFAULT_CLEARANCE_NM,
                    source: pcbcraft_engine::authoring::courtyard::CourtyardSource::PadsAndBody,
                    stroke_nm: pcbcraft_engine::authoring::courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        }
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut crate::PcbApp| app.frame(ui), app);
        h.run();
        let area = h.get_by_label("Library canvas").rect();
        let (cy_id, original_v0, v0_screen, target_screen, snapped_target) = {
            let doc = &h.state().library_editors.documents[&id];
            let item = doc.session.snapshot();
            let (cy_id, pts, _) = doc.selected_courtyard(&item).unwrap();
            let v0 = pts[0];
            let v0_screen = doc.camera.screen(v0, area);
            let target_screen = doc.camera.screen(Point { x_nm: v0.x_nm - 500_000, y_nm: v0.y_nm - 500_000 }, area);
            let snapped = doc.camera.snap(target_screen, area);
            assert!(area.contains(v0_screen) && area.contains(target_screen));
            assert_ne!(snapped, v0);
            assert!(pcbcraft_engine::authoring::courtyard::with_moved_vertex(&item.objects[&cy_id], 0, snapped).is_ok());
            (cy_id, v0, v0_screen, target_screen, snapped)
        };
        let rev = h.state().library_editors.documents[&id].session.revision();

        // Start dragging vertex 0 toward the target.
        h.hover_at(v0_screen);
        h.run();
        h.event(egui::Event::PointerButton { pos: v0_screen, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run();
        h.hover_at(target_screen);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, Tool::DragCourtyardVertex { vertex: 0, .. }));

        // Escape and pointer release in the same frame must cancel without committing.
        h.key_press(egui::Key::Escape);
        h.event(egui::Event::PointerButton {
            pos: target_screen,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run();
        {
            let doc = &h.state().library_editors.documents[&id];
            assert!(matches!(doc.tool, Tool::Select));
            assert_eq!(doc.selection.iter().copied().collect::<Vec<_>>(), vec![cy_id]);
            assert_eq!(doc.session.revision(), rev);
            let item = doc.session.snapshot();
            let (pts, _) = pcbcraft_engine::authoring::courtyard::courtyard_polygon(&item.objects[&cy_id]).unwrap();
            assert_eq!(pts[0], original_v0);
        }

        // Positive control: dragging vertex 0 without Escape commits the move in one edit.
        h.run_steps(30);
        h.hover_at(v0_screen);
        h.run();
        h.event(egui::Event::PointerButton { pos: v0_screen, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run();
        h.hover_at(target_screen);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, Tool::DragCourtyardVertex { vertex: 0, .. }));
        h.event(egui::Event::PointerButton {
            pos: target_screen,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run();
        let doc = &h.state().library_editors.documents[&id];
        assert!(matches!(doc.tool, Tool::Select));
        assert_eq!(doc.session.revision(), rev + 1);
        let item = doc.session.snapshot();
        let (pts, _) = pcbcraft_engine::authoring::courtyard::courtyard_polygon(&item.objects[&cy_id]).unwrap();
        assert_eq!(pts[0], snapped_target);
    }
}
