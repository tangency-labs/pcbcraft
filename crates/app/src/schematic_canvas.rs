mod camera;
mod wires;

pub(crate) use camera::Camera;

use crate::preferences::Navigation;
use egui::{Align2, FontId, Key, Modifiers, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use pcbcraft_chrome::{MenuItem, Tokens, menu, theme};
use pcbcraft_engine::{
    Project,
    schematic::{MAX_CENTER_NM, Point, SymbolDefinition, SymbolKind, SymbolPlacement, Terminal, pin_positions},
};

/// The active canvas interaction, bound to the revision at which it began.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Tool {
    PlaceSymbol { definition: &'static SymbolDefinition, revision: u64 },
    Wire { revision: u64, start: Option<Terminal> },
    Move { symbol: SymbolPlacement, offset: Vec2, revision: u64 },
}

impl Tool {
    /// Return the revision used to reject stale canvas actions.
    pub(crate) fn revision(self) -> u64 {
        match self {
            Self::PlaceSymbol { revision, .. } | Self::Wire { revision, .. } | Self::Move { revision, .. } => revision,
        }
    }
}

/// A placement, pin selection, or cancellation requested through the canvas.
pub(crate) enum Click {
    Place(Point),
    Pin(Terminal),
    CancelWire,
    Grab(SymbolPlacement, Vec2),
    FinishMove(Option<Point>),
}

/// Draw the schematic and its active preview, and report a requested tool action.
/// A primary drag with no active tool grabs the nearest symbol body. The move
/// preview ends on release, with no destination if released outside the canvas.
///
/// Return `Click::CancelWire` when Cancel or Escape dismisses an in-progress wire
/// through its context menu. An open wire menu can handle cancellation even
/// while canvas input is blocked.
///
/// With canvas input enabled and no drag in progress, a primary click during
/// placement returns `Click::Place` at a valid snapped point. In Wire mode, a
/// primary click that hits a terminal returns `Click::Pin`.
/// Return `None` for a preview without a valid click, an inactive tool, or
/// blocked or dragged canvas input when the wire menu did not cancel.
pub(crate) fn show(
    ui: &mut Ui,
    project: &Project,
    camera: &mut Camera,
    tool: Option<Tool>,
    input_enabled: bool,
    navigation: Navigation,
) -> Option<Click> {
    let rect = ui.available_rect_before_wrap();
    // Keep the canvas from taking keyboard focus away from editor shortcuts.
    let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Panel, true, "Schematic canvas"));
    let moving = matches!(tool, Some(Tool::Move { .. }));
    camera.navigate(&response, rect, project, input_enabled && !moving, navigation);

    let painter = ui.painter_at(rect);
    let tokens = Tokens::get();
    let screen = |point| camera.screen(point, rect);
    let origin = screen(Point { x_nm: 0, y_nm: 0 });
    let grid_step = camera.grid_step();
    paint_grid(&painter, rect, origin, tokens, grid_step);
    let moved = match tool {
        Some(Tool::Move { symbol, offset, .. }) => response
            .interact_pointer_pos()
            .filter(|pointer| rect.contains(*pointer))
            .map(|pointer| (symbol, camera.snap(pointer + offset, rect)))
            .filter(|(_, point)| valid_position(*point)),
        _ => None,
    };
    wires::paint(&painter, rect, project, camera, moved);
    let visible = rect.expand((grid_step * 5.0).max(theme::SCHEMATIC_FIT_MARGIN));
    for symbol in project.symbols() {
        let position = moved.filter(|(moving, _)| moving.terminals() == symbol.terminals()).map_or(symbol.position, |(_, point)| point);
        let center = screen(position);
        if visible.contains(center) {
            let reference = (grid_step >= theme::SCHEMATIC_MIN_GRID_SPACING).then(|| symbol.reference());
            let label = reference.as_deref().map(|reference| (reference, symbol.value()));
            paint_symbol(&painter, symbol.definition.kind, center, pin_positions(position).map(screen), tokens.text, label);
        }
    }

    if let Some(Tool::Wire { start: Some(start), .. }) = tool
        && (input_enabled || response.context_menu_opened())
        && let Some(popup) = menu::context(&response, |ui| {
            let cancel = MenuItem::new("Cancel").shortcut("Esc").show(ui);
            let escape = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
            if escape {
                ui.close();
            }
            cancel || escape
        })
    {
        if popup.inner {
            return Some(Click::CancelWire);
        }
        let anchor = egui::Popup::position_of_id(ui.ctx(), egui::Popup::default_response_id(&response));
        wires::preview(&painter, rect, project, camera, Some(start), anchor);
        return None;
    }

    if moving && response.drag_stopped_by(egui::PointerButton::Primary) {
        return Some(Click::FinishMove(moved.map(|(_, point)| point).filter(|_| input_enabled)));
    }
    if input_enabled
        && tool.is_none()
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
    {
        return grab_symbol(project, camera, rect, pointer);
    }

    if !input_enabled || response.dragged() {
        return None;
    }
    match tool? {
        Tool::PlaceSymbol { definition, .. } => placement_preview(&painter, rect, response, camera, tokens, definition.kind).map(Click::Place),
        Tool::Wire { start, .. } => wires::pick(&painter, rect, &response, project, camera, start).map(Click::Pin),
        Tool::Move { .. } => None,
    }
}

/// Pick the nearest symbol body under the original press and preserve the grab offset.
fn grab_symbol(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<Click> {
    let size = vec2(5.0, 4.0) * camera.grid_step();
    let (symbol, center) = project
        .symbols()
        .iter()
        .rev()
        .map(|symbol| (symbol, camera.screen(symbol.position, rect)))
        .filter(|(_, center)| Rect::from_center_size(*center, size).contains(pointer))
        .min_by(|(_, a), (_, b)| a.distance_sq(pointer).total_cmp(&b.distance_sq(pointer)))?;
    Some(Click::Grab(*symbol, center - pointer))
}

/// Keep previews inside the same coordinate bounds the command validates.
fn valid_position(point: Point) -> bool {
    (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.x_nm) && (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.y_nm)
}

/// Draw the pending symbol on its snapped model point until the user commits or cancels.
fn placement_preview(painter: &Painter, rect: Rect, response: Response, camera: &Camera, tokens: Tokens, kind: SymbolKind) -> Option<Point> {
    let screen = |point| camera.screen(point, rect);
    let pointer = response.hover_pos().or_else(|| response.interact_pointer_pos())?;
    let point = camera.snap(pointer, rect);
    if !valid_position(point) {
        return None;
    }
    paint_symbol(painter, kind, screen(point), pin_positions(point).map(screen), tokens.accent, None);
    response.on_hover_cursor(egui::CursorIcon::Crosshair).clicked_by(egui::PointerButton::Primary).then_some(point)
}

/// Draw visible grid intersections around the same origin used for snapping.
fn paint_grid(painter: &Painter, rect: Rect, origin: Pos2, tokens: Tokens, mut step: f32) {
    while step < theme::SCHEMATIC_MIN_GRID_SPACING {
        step *= 5.0;
    }
    let first_x = ((rect.left() - origin.x) / step).ceil() as i32;
    let last_x = ((rect.right() - origin.x) / step).floor() as i32;
    let first_y = ((rect.top() - origin.y) / step).ceil() as i32;
    let last_y = ((rect.bottom() - origin.y) / step).floor() as i32;
    for x in first_x..=last_x {
        for y in first_y..=last_y {
            let center = pos2(origin.x + x as f32 * step, origin.y + y as f32 * step);
            painter.circle_filled(center, theme::SCHEMATIC_GRID_DOT_RADIUS, tokens.canvas_grid);
        }
    }
}

const LED_ARROW_TIP_Y: f32 = -3.1;

/// Draw the symbol between its engine pin positions. At overview zoom use a simpler outline.
fn paint_symbol(painter: &Painter, kind: SymbolKind, center: Pos2, [left, right]: [Pos2; 2], color: egui::Color32, label: Option<(&str, &str)>) {
    let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color);
    let step = (right.x - left.x) / 10.0;
    let xy = |(x, y)| pos2(center.x + x * step, center.y + y * step);
    let line = |points: &[(f32, f32)]| painter.add(Shape::line(points.iter().copied().map(xy).collect(), stroke));
    if step < theme::SCHEMATIC_DETAIL_GRID_SPACING {
        match kind {
            SymbolKind::Resistor => {
                line(&[(-5.0, 0.0), (-2.5, 0.0), (-1.2, -0.9), (0.0, 0.9), (1.2, -0.9), (2.5, 0.0), (5.0, 0.0)]);
            }
            SymbolKind::Led => {
                line(&[(-5.0, 0.0), (-1.3, 0.0), (-1.3, -1.5), (1.3, 0.0), (-1.3, 1.5), (-1.3, 0.0)]);
                line(&[(1.3, -1.5), (1.3, 1.5), (1.3, 0.0), (5.0, 0.0)]);
            }
        }
        return;
    }
    painter.line_segment([left, xy((-2.5, 0.0))], stroke);
    painter.line_segment([xy((2.5, 0.0)), right], stroke);
    // The PCBcraft Next drawings are expressed in grid steps.
    let (reference_offset, value_offset) = match kind {
        SymbolKind::Resistor => {
            line(&[(-2.5, 0.0), (-2.0, -0.9), (-1.2, 0.9), (-0.4, -0.9), (0.4, 0.9), (1.2, -0.9), (2.0, 0.9), (2.5, 0.0)]);
            (1.2, 1.2)
        }
        SymbolKind::Led => {
            line(&[(-2.5, 0.0), (-1.3, 0.0)]);
            line(&[(-1.3, -1.5), (1.3, 0.0), (-1.3, 1.5), (-1.3, -1.5)]);
            line(&[(1.3, -1.5), (1.3, 1.5)]);
            line(&[(1.3, 0.0), (2.5, 0.0)]);
            line(&[(0.0, -2.0), (1.3, LED_ARROW_TIP_Y), (0.8, -3.0)]);
            line(&[(1.1, -1.7), (2.4, -2.8), (1.9, -2.7)]);
            (3.7, 1.8)
        }
    };
    if let Some((reference, value)) = label {
        let t = Tokens::get();
        let font = FontId::proportional(t.font_button);
        let label_offset = |grid_steps: f32| egui::vec2(0.0, (step * grid_steps).max(t.space_12));
        painter.text(center - label_offset(reference_offset), Align2::CENTER_BOTTOM, reference, font.clone(), color);
        painter.text(center + label_offset(value_offset), Align2::CENTER_TOP, value, font, color);
    }
}
