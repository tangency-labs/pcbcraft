use egui::{Event, MouseWheelUnit, PointerButton, Pos2, Rect, Response, Vec2, pos2, vec2};
use pcbcraft_chrome::theme;
use pcbcraft_engine::{
    Project,
    schematic::{GRID_NM, MAX_CENTER_NM, MAX_PIN_OFFSET_NM, Point},
    selection::Selection,
};

use super::grid_bounds;
use crate::preferences::{Navigation, ScrollAction, ZoomAround};

const MIN_ZOOM: f32 = 0.005;
const MAX_ZOOM: f32 = 8.0;

/// Viewport state in egui points. Navigation stays outside the project and its undo history.
#[derive(Debug)]
pub(crate) struct Camera {
    zoom: f32,
    offset: Vec2,
    fit_requested: bool,
    /// Usable canvas rectangle relative to its center, clear of floating cards.
    uncovered: Option<Rect>,
}

impl Default for Camera {
    fn default() -> Self {
        Self { zoom: 1.0, offset: Vec2::ZERO, fit_requested: false, uncovered: None }
    }
}

impl Camera {
    /// Seed another view from the currently displayed transform, leaving its host geometry
    /// unset. Splitting supersedes any unpainted Fit request so both views start at this zoom.
    pub(crate) fn fork_navigation(&mut self) -> Self {
        self.fit_requested = false;
        Self { zoom: self.zoom, offset: self.offset, ..Self::default() }
    }

    /// Reveal the selected symbols and wire paths if they extend outside the usable canvas.
    /// Zoom remains unchanged; oversized groups are centered even when they cannot fully fit.
    /// Empty selections, stale IDs and unavailable host geometry do not move the camera.
    pub(crate) fn reveal_selection(&mut self, project: &Project, selection: &Selection) {
        let mut bounds = Rect::NOTHING;
        let rect = Rect::from_center_size(Pos2::ZERO, Vec2::ZERO);
        for symbol in project.symbols().iter().filter(|symbol| selection.symbols.contains(&symbol.id())) {
            let center = self.screen(symbol.position, rect).to_vec2();
            let extent = grid_bounds(&symbol.definition, symbol.turns());
            let step = self.grid_step();
            bounds = bounds.union(
                Rect::from_min_max(pos2(extent.min.x * step, extent.min.y * step), pos2(extent.max.x * step, extent.max.y * step)).translate(center),
            );
        }
        for point in project.wires().iter().filter(|wire| selection.wires.contains(&wire.id())).flat_map(|wire| project.wire_path(wire)) {
            bounds = bounds.union(Rect::from_pos(self.screen(point, rect)));
        }
        self.reveal_bounds(bounds);
    }

    /// Center already projected bounds only when they are outside the usable region. Keep a
    /// label margin where space permits; a visible selection never nudges the camera.
    fn reveal_bounds(&mut self, bounds: Rect) {
        if let Some(uncovered) = self.uncovered
            && uncovered.is_positive()
            && bounds.is_finite()
        {
            let margin = theme::SCHEMATIC_FIT_MARGIN.min(uncovered.width() / 4.0).min(uncovered.height() / 4.0);
            if !uncovered.shrink(margin).contains_rect(bounds) {
                self.offset += uncovered.center() - bounds.center();
                self.fit_requested = false;
            }
        }
    }

    /// Fit the project after the menu closes, using the next canvas rectangle.
    pub(crate) fn request_fit(&mut self) {
        self.fit_requested = true;
    }

    /// Set the rectangle available to Fit and centering, relative to the full canvas. Opening
    /// or moving a card changes this target without moving the camera until navigation is requested.
    pub(crate) fn set_uncovered(&mut self, canvas: Rect, uncovered: Rect) {
        self.uncovered = Some(uncovered.translate(-canvas.center().to_vec2()));
    }

    /// The canvas offset that puts the drawing's origin at the middle of the uncovered part.
    fn uncovered_center(&self) -> Vec2 {
        self.uncovered.map_or(Vec2::ZERO, |rect| rect.center().to_vec2())
    }

    /// Pan so a model point sits at the center of the uncovered canvas, keeping the zoom.
    /// Cancels a pending fit, which would otherwise move the view away again.
    pub(crate) fn center_on(&mut self, point: Point) {
        let scale = self.grid_step() / GRID_NM as f32;
        self.offset = self.uncovered_center() - vec2(point.x_nm as f32, point.y_nm as f32) * scale;
        self.fit_requested = false;
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
    pub(crate) fn screen(&self, point: Point, rect: Rect) -> Pos2 {
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
        let limit = Vec2::splat((MAX_CENTER_NM + MAX_PIN_OFFSET_NM) as f32 / GRID_NM as f32 * self.grid_step());
        self.offset = self.offset.clamp(-limit, limit);
    }

    /// Center symbol and wire bounds, including free routes and a margin for labels, in the part
    /// of the canvas left uncovered by all visible floating cards.
    fn fit(&mut self, project: &Project, rect: Rect) {
        *self = Self { uncovered: self.uncovered, ..Self::default() };
        let mut bounds = Rect::NOTHING;
        for symbol in project.symbols() {
            let center = self.screen(symbol.position, rect) - rect.center();
            let extent = grid_bounds(&symbol.definition, symbol.turns());
            let step = self.grid_step();
            let symbol_bounds = Rect::from_min_max(pos2(extent.min.x * step, extent.min.y * step), pos2(extent.max.x * step, extent.max.y * step))
                .translate(center)
                .expand(theme::SCHEMATIC_FIT_MARGIN);
            bounds = bounds.union(symbol_bounds);
        }
        for point in project.wires().iter().flat_map(|wire| project.wire_path(wire)) {
            bounds = bounds.union(Rect::from_pos((self.screen(point, rect) - rect.center()).to_pos2()).expand(theme::SCHEMATIC_FIT_MARGIN));
        }
        if bounds.is_finite() {
            let size = self.uncovered.map_or(rect.size(), |clear| clear.size());
            let available = (size - Vec2::splat(2.0 * theme::SCHEMATIC_FIT_MARGIN)).max(Vec2::splat(1.0));
            self.zoom = (available.x / bounds.width()).min(available.y / bounds.height()).clamp(MIN_ZOOM, 1.0);
            self.offset = self.uncovered_center() - bounds.center().to_vec2() * self.zoom;
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

    /// Visible targets do not move, while out-of-view and oversized bounds center in the
    /// uncovered region without zooming. Missing or invalid geometry cannot move the view.
    #[test]
    fn reveal_respects_uncovered_geometry_without_changing_zoom() {
        let mut camera = Camera { zoom: 2.0, offset: vec2(12.0, 30.0), ..Camera::default() };
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let clear = Rect::from_min_size(pos2(300.0, 0.0), vec2(500.0, 600.0));
        camera.set_uncovered(canvas, clear);
        let usable = camera.uncovered.unwrap();
        let visible = Rect::from_center_size(usable.center(), vec2(40.0, 40.0));
        camera.reveal_bounds(visible);
        assert_eq!(camera.offset, vec2(12.0, 30.0));
        let outside = visible.translate(vec2(-600.0, 800.0));
        camera.reveal_bounds(outside);
        assert_eq!(camera.offset, vec2(612.0, -770.0));
        let oversized = Rect::from_center_size(pos2(500.0, 400.0), vec2(1000.0, 1000.0));
        let before = camera.offset;
        camera.reveal_bounds(oversized);
        assert_eq!(camera.offset, before + (usable.center() - oversized.center()));
        let before = camera.offset;
        camera.reveal_bounds(Rect::NOTHING);
        camera.uncovered = None;
        camera.reveal_bounds(outside);
        assert_eq!(camera.offset, before);
        assert_eq!(camera.zoom, 2.0);
    }

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
