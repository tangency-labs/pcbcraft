use egui::{Event, MouseWheelUnit, PointerButton, Pos2, Rect, Response, Vec2, pos2, vec2};
use pcbcraft_chrome::theme;
use pcbcraft_engine::{
    Project,
    schematic::{GRID_NM, MAX_CENTER_NM, PIN_OFFSET_NM, Point},
};

use super::top_extent;
use crate::preferences::{Navigation, ScrollAction, ZoomAround};

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

    /// The current zoom factor, for tests that check navigation.
    #[cfg(test)]
    pub(crate) fn zoom(&self) -> f32 {
        self.zoom
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

    /// Apply navigation only when the canvas owns input, following the person's settings.
    /// The zoom anchor (the pointer, or the view center) stays fixed while zooming. When
    /// scrolling pans, Ctrl or ⌘ with the wheel still zooms, and a pinch always zooms.
    pub(super) fn navigate(&mut self, response: &Response, rect: Rect, project: &Project, enabled: bool, navigation: Navigation) {
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
            let Wheel { zoom, pan } = response.ctx.input(|input| wheel(&input.events, rect.height(), navigation));
            self.offset += pan;
            if zoom != 0.0 {
                let anchor = match navigation.zoom_around {
                    ZoomAround::Pointer => pointer,
                    ZoomAround::Center => rect.center(),
                };
                let before = (anchor - rect.center() - self.offset) / self.zoom;
                self.zoom = (self.zoom * zoom.clamp(-2.0, 2.0).exp()).clamp(MIN_ZOOM, MAX_ZOOM);
                self.offset = anchor - rect.center() - before * self.zoom;
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
            let top = left.y + top_extent(symbol.definition.kind) * self.grid_step();
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

/// This frame's wheel and pinch input: a logarithmic zoom delta and a pan in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Wheel {
    zoom: f32,
    pan: Vec2,
}

/// Normalize mouse-wheel units and pinch gestures, then apply the navigation settings.
/// Speed and direction change wheel zoom only; a pinch follows the fingers.
fn wheel(events: &[Event], viewport_height: f32, navigation: Navigation) -> Wheel {
    let direction = if navigation.reverse_zoom { -1.0 } else { 1.0 };
    let mut out = Wheel::default();
    for event in events {
        match event {
            Event::MouseWheel { unit, delta, modifiers, .. } if delta.is_finite() => {
                let scale = match unit {
                    MouseWheelUnit::Point => 1.0,
                    MouseWheelUnit::Line => 40.0,
                    MouseWheelUnit::Page => viewport_height,
                };
                if navigation.scroll == ScrollAction::Pan && !modifiers.command {
                    out.pan += *delta * scale;
                } else {
                    out.zoom += (delta.y * scale * 0.004 * navigation.zoom_speed()).clamp(-1.5, 1.5) * direction;
                }
            }
            Event::Zoom(factor) if factor.is_finite() && *factor > 0.0 => out.zoom += factor.ln().clamp(-1.5, 1.5),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scroll(y: f32, command: bool) -> Event {
        let modifiers = if command { egui::Modifiers::COMMAND } else { egui::Modifiers::NONE };
        Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, y), modifiers, phase: egui::TouchPhase::Move }
    }

    #[test]
    fn default_navigation_zooms_with_the_wheel() {
        let input = wheel(&[scroll(50.0, false)], 500.0, Navigation::default());
        assert!(input.zoom > 0.0);
        assert_eq!(input.pan, Vec2::ZERO);
    }

    #[test]
    fn reverse_and_speed_change_wheel_zoom_but_not_pinch() {
        let base = wheel(&[scroll(50.0, false)], 500.0, Navigation::default()).zoom;
        let reversed = Navigation { reverse_zoom: true, ..Navigation::default() };
        assert_eq!(wheel(&[scroll(50.0, false)], 500.0, reversed).zoom, -base);
        let fast = Navigation { zoom_speed_percent: 200, ..Navigation::default() };
        assert!((wheel(&[scroll(50.0, false)], 500.0, fast).zoom - 2.0 * base).abs() < 1e-6);
        let pinch = [Event::Zoom(1.5)];
        assert_eq!(wheel(&pinch, 500.0, reversed).zoom, wheel(&pinch, 500.0, Navigation::default()).zoom);
    }

    #[test]
    fn panning_scroll_moves_the_view_and_ctrl_still_zooms() {
        let pan = Navigation { scroll: ScrollAction::Pan, ..Navigation::default() };
        let moved = wheel(&[scroll(-30.0, false)], 500.0, pan);
        assert_eq!((moved.zoom, moved.pan), (0.0, vec2(0.0, -30.0)));
        assert!(wheel(&[scroll(30.0, true)], 500.0, pan).zoom > 0.0);
    }
}
