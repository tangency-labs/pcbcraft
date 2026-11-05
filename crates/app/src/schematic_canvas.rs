use egui::{Align2, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2};
use pcbcraft_chrome::{Tokens, theme};
use pcbcraft_engine::{
    Project,
    schematic::{GRID_NM, MAX_CENTER_NM, PIN_OFFSET_NM, Point},
};

pub(crate) fn show(ui: &mut Ui, project: &Project, placing: bool) -> Option<Point> {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, if placing { Sense::click() } else { Sense::hover() });
    let painter = ui.painter_at(rect);
    let origin = rect.center();
    let tokens = Tokens::get();
    paint_grid(&painter, rect, origin, tokens);
    let screen = |point: Point| {
        let scale = theme::SCHEMATIC_GRID_SPACING / GRID_NM as f32;
        pos2(origin.x + point.x_nm as f32 * scale, origin.y + point.y_nm as f32 * scale)
    };
    for resistor in project.resistors() {
        let center = screen(resistor.position);
        if rect.expand(theme::SCHEMATIC_GRID_SPACING * 5.0).contains(center) {
            let [left, right] = resistor.pins().map(screen);
            paint_resistor(&painter, center, [left, right], tokens.text, Some((&resistor.reference(), resistor.value())));
        }
    }

    if !placing {
        return None;
    }
    let pointer = response.hover_pos().or_else(|| response.interact_pointer_pos())?;
    let snap = |offset: f32| (offset / theme::SCHEMATIC_GRID_SPACING).round() as i64 * GRID_NM;
    let point = Point { x_nm: snap(pointer.x - origin.x), y_nm: snap(pointer.y - origin.y) };
    if !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.x_nm) || !(-MAX_CENTER_NM..=MAX_CENTER_NM).contains(&point.y_nm) {
        return None;
    }
    let left = Point { x_nm: point.x_nm - PIN_OFFSET_NM, ..point };
    let right = Point { x_nm: point.x_nm + PIN_OFFSET_NM, ..point };
    paint_resistor(&painter, screen(point), [screen(left), screen(right)], tokens.accent, None);
    response.on_hover_cursor(egui::CursorIcon::Crosshair).clicked().then_some(point)
}

fn paint_grid(painter: &Painter, rect: Rect, origin: Pos2, tokens: Tokens) {
    let step = theme::SCHEMATIC_GRID_SPACING;
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

fn paint_resistor(painter: &Painter, center: Pos2, [left, right]: [Pos2; 2], color: egui::Color32, label: Option<(&str, &str)>) {
    let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color);
    // Zigzag from PCBcraft Next, expressed in grid steps.
    let outline = [(-2.5, 0.0), (-2.0, -0.9), (-1.2, 0.9), (-0.4, -0.9), (0.4, 0.9), (1.2, -0.9), (2.0, 0.9), (2.5, 0.0)];
    let points: Vec<_> =
        outline.into_iter().map(|(x, y)| pos2(center.x + x * theme::SCHEMATIC_GRID_SPACING, center.y + y * theme::SCHEMATIC_GRID_SPACING)).collect();
    painter.line_segment([left, points[0]], stroke);
    painter.line_segment([points[points.len() - 1], right], stroke);
    painter.add(Shape::line(points, stroke));
    if let Some((reference, value)) = label {
        let t = Tokens::get();
        let font = FontId::proportional(t.font_button);
        painter.text(center - egui::vec2(0.0, t.space_12), Align2::CENTER_BOTTOM, reference, font.clone(), color);
        painter.text(center + egui::vec2(0.0, t.space_12), Align2::CENTER_TOP, value, font, color);
    }
}
