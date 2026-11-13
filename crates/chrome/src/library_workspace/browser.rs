//! Browser sizing uses the same accessible edge control and gesture state as Preview.
use super::column::{ColumnWidth, Sash};
use crate::Tokens;
use egui::{Rect, Ui, pos2};

/// Pane-local layout remembers its requested width when the host temporarily becomes compact.
#[derive(Clone, Debug, Default)]
pub struct Browser {
    pub collapsed: bool,
    column: ColumnWidth,
}
impl Browser {
    /// Requested logical width, independently of temporary host constraints.
    pub fn width(&self) -> f32 {
        self.column.width(230.0)
    }
    /// Explicit width changes validate atomically and end any previous drag.
    pub fn set_width(&mut self, width: f32) -> Result<(), String> {
        self.column.set_width(width)
    }
    /// Compact hosts restore a pending drag's original preference.
    pub(super) fn cancel_resize(&mut self) {
        self.column.cancel_resize();
    }
    /// The quiet shared sash owns only the gap at the Browser's right edge.
    pub(super) fn resize(&mut self, ui: &mut Ui, area: Rect, max: f32) -> f32 {
        let max = max.clamp(180.0, 640.0);
        let t = Tokens::get();
        let x = area.left() + self.width().clamp(180.0, max) + t.space_12 / 2.0;
        let bounds = Rect::from_min_max(pos2(x - t.space_6 / 2.0, area.top()), pos2(x + t.space_6 / 2.0, area.bottom()));
        self.column.resize(
            ui,
            ui.make_persistent_id("library_browser_width"),
            230.0,
            Sash { bounds, name: "Resize Browser width", min: 180.0, max, leading: false },
        )
    }
}
