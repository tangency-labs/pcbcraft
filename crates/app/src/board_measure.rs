//! Inspect › Measure: a view-only ruler between two snapped board points. It never edits the
//! project and is not saved; Esc clears the measurement, then leaves the tool.
use crate::canvas_camera::Camera;
use egui::{PointerButton, Rect, Response, Ui};
use pcbcraft_chrome::units;
use pcbcraft_engine::{board::view::BoardView, schematic::Point};

/// Measure tool state for one board view.
#[derive(Debug, Default)]
pub(super) struct Measure {
    pub(super) tool: bool,
    start: Option<Point>,
    /// The pinned far end; while unset, the ruler follows the snapped pointer.
    end: Option<Point>,
}

impl Measure {
    /// Arm the tool with no measurement yet.
    pub(super) fn start(&mut self) {
        *self = Self { tool: true, ..Self::default() };
    }
    /// Leave the tool and drop any measurement.
    pub(super) fn cancel(&mut self) {
        *self = Self::default();
    }
    /// Esc clears a measurement first, then leaves the tool. Returns false when idle.
    pub(super) fn escape(&mut self) -> bool {
        if self.start.take().is_some() {
            self.end = None;
            return true;
        }
        std::mem::take(&mut self.tool)
    }
    /// Whether the tool owns canvas input.
    pub(super) fn busy(&self) -> bool {
        self.tool
    }
}

/// Handle clicks while the tool is armed: the first click sets the start, the second pins the
/// end, and a third starts over. Right-click leaves the tool. Returns whether input was used.
pub(super) fn interact(ui: &Ui, response: &Response, camera: &Camera, view: BoardView<'_>, state: &mut Measure, enabled: bool) -> bool {
    if !state.tool {
        return false;
    }
    if !enabled {
        return true;
    }
    if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) {
        state.cancel();
        return true;
    }
    let rect = response.rect;
    if response.clicked()
        && let Some(pointer) = ui.input(|input| input.pointer.hover_pos()).filter(|point| rect.contains(*point))
    {
        let at = crate::board_tracks::copper_target(pointer, camera, rect, view);
        match (state.start, state.end) {
            (Some(_), None) => state.end = Some(at),
            _ => {
                state.start = Some(at);
                state.end = None;
            }
        }
    }
    true
}

/// The distance text for a ruler: its length, then the horizontal and vertical components.
pub(super) fn label(a: Point, b: Point, unit: units::Unit) -> String {
    let (dx, dy) = (b.x_nm - a.x_nm, b.y_nm - a.y_nm);
    let length = (dx as f64).hypot(dy as f64).round() as i64;
    format!("{} · Δx {} · Δy {}", units::label(length, unit), units::label(dx, unit), units::label(-dy, unit))
}

/// Paint the ruler: pinned, or following the snapped pointer while the far end is open.
pub(super) fn paint(ui: &Ui, rect: Rect, camera: &Camera, view: BoardView<'_>, state: &Measure, unit: units::Unit) {
    if !state.tool {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    let pointer = ui.input(|input| input.pointer.hover_pos()).filter(|point| rect.contains(*point));
    let target = pointer.map(|pointer| crate::board_tracks::copper_target(pointer, camera, rect, view));
    if let Some(target) = target {
        pcbcraft_chrome::board::drawing_target(&painter, camera.screen(target, rect));
    }
    let Some(start) = state.start else { return };
    let Some(end) = state.end.or(target) else { return };
    pcbcraft_chrome::board::measurement(&painter, camera.screen(start, rect), camera.screen(end, rect), &label(start, end, unit));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The label reports length and components with Y up, in the chosen unit.
    #[test]
    fn labels_report_length_and_components() {
        let a = Point { x_nm: 0, y_nm: 0 };
        let b = Point { x_nm: 3_000_000, y_nm: -4_000_000 };
        assert_eq!(label(a, b, units::Unit::Mm), "5 mm · Δx 3 mm · Δy 4 mm");
    }
}
