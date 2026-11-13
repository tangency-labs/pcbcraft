//! Browser sizing uses the same accessible edge control as workspace panels.
use crate::Tokens;
use egui::{Rect, Ui, pos2};

/// Pane-local layout remembers its requested width when the host temporarily becomes compact.
#[derive(Debug)]
pub struct Browser {
    pub collapsed: bool,
    width: f32,
    resize: Option<Resize>,
}
#[derive(Debug)]
struct Resize {
    pointer: f32,
    start: f32,
    before: f32,
}
impl Default for Browser {
    fn default() -> Self {
        Self { collapsed: false, width: 230.0, resize: None }
    }
}
impl Clone for Browser {
    /// Opening beside copies layout preferences without transferring an active pointer gesture.
    fn clone(&self) -> Self {
        Self { collapsed: self.collapsed, width: self.width, resize: None }
    }
}
impl Browser {
    /// Requested logical width, independently of temporary host constraints.
    pub fn width(&self) -> f32 {
        self.width
    }
    /// Explicit width changes validate atomically and end any previous drag.
    pub fn set_width(&mut self, width: f32) -> Result<(), String> {
        if !width.is_finite() || !(180.0..=640.0).contains(&width) {
            return Err("Browser width must be between 180 and 640 logical points".into());
        }
        self.width = width;
        self.resize = None;
        Ok(())
    }
    /// Escape, focus loss and compact layouts restore a pending drag's original preference.
    pub(super) fn cancel_resize(&mut self) {
        if let Some(resize) = self.resize.take() {
            self.width = resize.before;
        }
    }
    /// A quiet shared sash owns only the gap at the Browser's right edge. Pointer and keyboard
    /// changes reserve usable Items and Preview space; no horizontal scrollbar is introduced.
    pub(super) fn resize(&mut self, ui: &mut Ui, area: Rect, max: f32) -> f32 {
        let max = max.clamp(180.0, 640.0);
        if !ui.is_enabled()
            || ui.input(|input| {
                input.key_pressed(egui::Key::Escape) || input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false)))
            })
        {
            self.cancel_resize();
            return self.width.clamp(180.0, max);
        }
        let t = Tokens::get();
        let width = self.width.clamp(180.0, max);
        let x = area.left() + width + t.space_12 / 2.0;
        let bounds = Rect::from_min_max(pos2(x - t.space_6 / 2.0, area.top()), pos2(x + t.space_6 / 2.0, area.bottom()));
        let response =
            solidcraft_ui_kit::panel_host_sash(ui, ui.make_persistent_id("library_browser_width"), bounds, "Resize Browser width", false, width);
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if response.drag_started()
            && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
        {
            self.resize = Some(Resize { pointer: pointer.x, start: width, before: self.width });
        }
        let mut next = None;
        if response.dragged()
            && let Some(resize) = &self.resize
            && let Some(pointer) = response.interact_pointer_pos()
        {
            next = Some(resize.start + pointer.x - resize.pointer);
        }
        if response.has_focus() {
            let delta = ui.input_mut(|input| {
                (input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) as i8
                    - input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) as i8) as f32
                    * t.space_16
            });
            if delta != 0.0 {
                next = Some(width + delta);
            }
        }
        if let Some(next) = next {
            let next = next.clamp(180.0, max);
            if self.width != next {
                self.width = next;
                ui.ctx().request_repaint();
            }
        }
        if response.drag_stopped() {
            self.resize = None;
        }
        self.width.clamp(180.0, max)
    }
}
