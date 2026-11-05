mod camera;
mod wires;

pub(crate) use camera::Camera;

use egui::{Align2, FontId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, pos2};
use pcbcraft_chrome::{Tokens, theme};
use pcbcraft_engine::{
    Project,
    schematic::{MAX_CENTER_NM, PIN_OFFSET_NM, Point, Terminal},
};

/// The active canvas interaction, bound to the revision at which it began.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Tool {
    PlaceResistor { revision: u64 },
    Wire { revision: u64, start: Option<Terminal> },
}

impl Tool {
    /// Return the revision used to reject stale canvas actions.
    pub(crate) fn revision(self) -> u64 {
        match self {
            Self::PlaceResistor { revision } | Self::Wire { revision, .. } => revision,
        }
    }
}

/// An exact click target chosen through the current tool.
pub(crate) enum Click {
    Place(Point),
    Pin(Terminal),
}

/// Draw the schematic and its active preview, returning a target only on a canvas click.
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
    for resistor in project.resistors() {
        let center = screen(resistor.position);
        if rect.expand((grid_step * 5.0).max(theme::SCHEMATIC_FIT_MARGIN)).contains(center) {
            let [left, right] = resistor.pins().map(screen);
            let reference = resistor.reference();
            let label = (grid_step >= theme::SCHEMATIC_MIN_GRID_SPACING).then_some((reference.as_str(), resistor.value()));
            paint_resistor(&painter, center, [left, right], tokens.text, label);
        }
    }

    if !input_enabled || response.dragged() {
        return None;
    }
    match tool? {
        Tool::PlaceResistor { .. } => placement_preview(&painter, rect, response, camera, tokens).map(Click::Place),
        Tool::Wire { start, .. } => wires::pick(&painter, rect, &response, project, camera, start).map(Click::Pin),
    }
}

/// Draw the resistor on its snapped model point until the user commits or cancels.
fn placement_preview(painter: &Painter, rect: Rect, response: Response, camera: &Camera, tokens: Tokens) -> Option<Point> {
    let screen = |point| camera.screen(point, rect);
    let pointer = response.hover_pos().or_else(|| response.interact_pointer_pos())?;
    let point = camera.snap(pointer, rect);
    if !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.x_nm) || !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.y_nm) {
        return None;
    }
    let left = Point { x_nm: point.x_nm - PIN_OFFSET_NM, ..point };
    let right = Point { x_nm: point.x_nm + PIN_OFFSET_NM, ..point };
    paint_resistor(painter, screen(point), [screen(left), screen(right)], tokens.accent, None);
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

/// Draw the resistor between its engine pin positions. A placement preview omits the labels.
fn paint_resistor(painter: &Painter, center: Pos2, [left, right]: [Pos2; 2], color: egui::Color32, label: Option<(&str, &str)>) {
    let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color);
    // Zigzag from PCBcraft Next, expressed in grid steps.
    let outline = [(-2.5, 0.0), (-2.0, -0.9), (-1.2, 0.9), (-0.4, -0.9), (0.4, 0.9), (1.2, -0.9), (2.0, 0.9), (2.5, 0.0)];
    let step = (right.x - left.x) / 10.0;
    let points: Vec<_> = outline.into_iter().map(|(x, y)| pos2(center.x + x * step, center.y + y * step)).collect();
    painter.line_segment([left, points[0]], stroke);
    painter.line_segment([points[points.len() - 1], right], stroke);
    painter.add(Shape::line(points, stroke));
    if let Some((reference, value)) = label {
        let t = Tokens::get();
        let font = FontId::proportional(t.font_button);
        let label_offset = egui::vec2(0.0, (step * 1.2).max(t.space_12));
        painter.text(center - label_offset, Align2::CENTER_BOTTOM, reference, font.clone(), color);
        painter.text(center + label_offset, Align2::CENTER_TOP, value, font, color);
    }
}
