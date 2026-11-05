use egui::{Event, MouseWheelUnit, PointerButton, Pos2, Rect, Response, Vec2, pos2, vec2};
use pcbcraft_chrome::theme;
use pcbcraft_engine::{
    Project,
    schematic::{GRID_NM, MAX_CENTER_NM, PIN_OFFSET_NM, Point, SymbolKind},
};

use super::LED_ARROW_TIP_Y;

const MIN_ZOOM: f32 = 0.005;
const MAX_ZOOM: f32 = 8.0;

/// Viewport state in egui points. Navigation stays outside the project and its undo history.
#[derive(Debug)]
pub(crate) struct Camera {
    zoom: f32,
    offset: Vec2,
    fit_requested: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self { zoom: 1.0, offset: Vec2::ZERO, fit_requested: false }
    }
}

impl Camera {
    /// Fit the project after the menu closes, using the next canvas rectangle.
    pub(crate) fn request_fit(&mut self) {
        self.fit_requested = true;
    }

    /// The visible spacing of the underlying placement grid, before coarsening its drawing.
    pub(super) fn grid_step(&self) -> f32 {
        theme::SCHEMATIC_GRID_SPACING * self.zoom
    }

    /// Convert a model point to a point on the canvas.
    pub(super) fn screen(&self, point: Point, rect: Rect) -> Pos2 {
        let scale = self.grid_step() / GRID_NM as f32;
        rect.center() + self.offset + vec2(point.x_nm as f32, point.y_nm as f32) * scale
    }

    /// Snap the pointer in schematic coordinates, even when the displayed grid is coarser.
    pub(super) fn snap(&self, pointer: Pos2, rect: Rect) -> Point {
        let relative = (pointer - rect.center() - self.offset) / self.grid_step();
        Point { x_nm: relative.x.round() as i64 * GRID_NM, y_nm: relative.y.round() as i64 * GRID_NM }
    }

    /// Apply navigation only when the canvas owns input. Wheel and pinch keep their anchor fixed.
    pub(super) fn navigate(&mut self, response: &Response, rect: Rect, project: &Project, enabled: bool) {
        if std::mem::take(&mut self.fit_requested) {
            self.fit(project, rect);
        }
        if !enabled {
            return;
        }
        if response.dragged_by(PointerButton::Middle) {
            self.offset += response.drag_delta();
        }
        if response.contains_pointer()
            && let Some(pointer) = response.ctx.input(|i| i.pointer.hover_pos())
        {
            let exponent = response.ctx.input(|input| wheel_exponent(&input.events, rect.height()));
            if exponent != 0.0 {
                let before = (pointer - rect.center() - self.offset) / self.zoom;
                self.zoom = (self.zoom * exponent.clamp(-2.0, 2.0).exp()).clamp(MIN_ZOOM, MAX_ZOOM);
                self.offset = pointer - rect.center() - before * self.zoom;
            }
        }
        // Keep the view center within reach of the full schematic coordinate range.
        let limit = Vec2::splat((MAX_CENTER_NM + PIN_OFFSET_NM) as f32 / GRID_NM as f32 * self.grid_step());
        self.offset = self.offset.clamp(-limit, limit);
    }

    /// Center the bounds of the symbols and their leads, including a margin for labels.
    fn fit(&mut self, project: &Project, rect: Rect) {
        *self = Self::default();
        let mut bounds = Rect::NOTHING;
        for symbol in project.symbols() {
            let [left, right] = symbol.pins().map(|pin| self.screen(pin, rect) - rect.center());
            let top = match symbol.definition.kind {
                SymbolKind::Resistor => left.y,
                SymbolKind::Led => left.y + LED_ARROW_TIP_Y * self.grid_step(),
            };
            let symbol_bounds = Rect::from_min_max(pos2(left.x, top), pos2(right.x, right.y)).expand(theme::SCHEMATIC_FIT_MARGIN);
            bounds = bounds.union(symbol_bounds);
        }
        if bounds.is_finite() {
            let available = (rect.size() - Vec2::splat(2.0 * theme::SCHEMATIC_FIT_MARGIN)).max(Vec2::splat(1.0));
            self.zoom = (available.x / bounds.width()).min(available.y / bounds.height()).clamp(MIN_ZOOM, 1.0);
            self.offset = -bounds.center().to_vec2() * self.zoom;
        }
    }
}

/// Normalize mouse-wheel units and pinch gestures to a logarithmic zoom delta.
fn wheel_exponent(events: &[Event], viewport_height: f32) -> f32 {
    events
        .iter()
        .map(|event| match event {
            Event::MouseWheel { unit, delta, .. } if delta.y.is_finite() => {
                let scale = match unit {
                    MouseWheelUnit::Point => 1.0,
                    MouseWheelUnit::Line => 40.0,
                    MouseWheelUnit::Page => viewport_height,
                };
                (delta.y * scale * 0.004).clamp(-1.5, 1.5)
            }
            Event::Zoom(factor) if factor.is_finite() && *factor > 0.0 => factor.ln().clamp(-1.5, 1.5),
            _ => 0.0,
        })
        .sum()
}
