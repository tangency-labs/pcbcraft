mod camera;
mod wires;

pub(crate) use camera::Camera;

use egui::{Align2, FontId, Key, Modifiers, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, pos2};
use pcbcraft_chrome::{MenuItem, Tokens, menu, theme};
use pcbcraft_engine::{
    Project,
    schematic::{MAX_CENTER_NM, Point, SymbolKind, Terminal, pin_positions},
};

/// The active canvas interaction, bound to the revision at which it began.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Tool {
    PlaceSymbol { kind: SymbolKind, revision: u64 },
    Wire { revision: u64, start: Option<Terminal> },
}

impl Tool {
    /// Return the revision used to reject stale canvas actions.
    pub(crate) fn revision(self) -> u64 {
        match self {
            Self::PlaceSymbol { revision, .. } | Self::Wire { revision, .. } => revision,
        }
    }
}

/// A placement, pin selection, or cancellation requested through the canvas.
pub(crate) enum Click {
    Place(Point),
    Pin(Terminal),
    CancelWire,
}

/// Draw the schematic and active preview, then return any canvas tool action.
pub(crate) fn show(ui: &mut Ui, project: &Project, camera: &mut Camera, tool: Option<Tool>, input_enabled: bool) -> Option<Click> {
    let rect = ui.available_rect_before_wrap();
    // Keep the canvas from taking keyboard focus away from editor shortcuts.
    let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
    camera.navigate(&response, rect, project, input_enabled);

    let painter = ui.painter_at(rect);
    let tokens = Tokens::get();
    let screen = |point| camera.screen(point, rect);
    let origin = screen(Point { x_nm: 0, y_nm: 0 });
    let grid_step = camera.grid_step();
    paint_grid(&painter, rect, origin, tokens, grid_step);
    wires::paint(&painter, rect, project, camera);
    for symbol in project.symbols() {
        let center = screen(symbol.position);
        if rect.expand((grid_step * 5.0).max(theme::SCHEMATIC_FIT_MARGIN)).contains(center) {
            let reference = symbol.reference();
            let label = (grid_step >= theme::SCHEMATIC_MIN_GRID_SPACING).then_some((reference.as_str(), symbol.value()));
            paint_symbol(&painter, symbol.kind, center, symbol.pins().map(screen), tokens.text, label);
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

    if !input_enabled || response.dragged() {
        return None;
    }
    match tool? {
        Tool::PlaceSymbol { kind, .. } => placement_preview(&painter, rect, response, camera, tokens, kind).map(Click::Place),
        Tool::Wire { start, .. } => wires::pick(&painter, rect, &response, project, camera, start).map(Click::Pin),
    }
}

/// Draw the pending symbol on its snapped model point until the user commits or cancels.
fn placement_preview(painter: &Painter, rect: Rect, response: Response, camera: &Camera, tokens: Tokens, kind: SymbolKind) -> Option<Point> {
    let screen = |point| camera.screen(point, rect);
    let pointer = response.hover_pos().or_else(|| response.interact_pointer_pos())?;
    let point = camera.snap(pointer, rect);
    if !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.x_nm) || !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.y_nm) {
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

/// Draw the symbol between its engine pin positions. A placement preview omits the labels.
fn paint_symbol(painter: &Painter, kind: SymbolKind, center: Pos2, [left, right]: [Pos2; 2], color: egui::Color32, label: Option<(&str, &str)>) {
    let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color);
    let step = (right.x - left.x) / 10.0;
    let xy = |(x, y)| pos2(center.x + x * step, center.y + y * step);
    let line = |points: &[(f32, f32)]| painter.add(Shape::line(points.iter().copied().map(xy).collect(), stroke));
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
