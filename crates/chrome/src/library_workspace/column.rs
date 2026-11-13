//! Controlled column extents share the toolkit's accessible sash and gesture lifecycle.
use crate::Tokens;
use egui::{Id, Rect, Ui};

/// Optional preferred width lets an untouched column keep its responsive host default.
#[derive(Debug, Default)]
pub struct ColumnWidth {
    preferred: Option<f32>,
    resize: Option<Resize>,
}
#[derive(Debug)]
struct Resize {
    pointer: f32,
    start: f32,
    before: Option<f32>,
}
/// Actual host bounds and edge orientation; drawing and commands remain host-owned.
pub(super) struct Sash<'a> {
    pub bounds: Rect,
    pub name: &'a str,
    pub min: f32,
    pub max: f32,
    pub leading: bool,
}
impl Clone for ColumnWidth {
    /// Cloned panes retain preferred layout without taking ownership of another pane's gesture.
    fn clone(&self) -> Self {
        Self { preferred: self.preferred, resize: None }
    }
}
impl ColumnWidth {
    /// Resolve the requested width against the host's default, before its current size bounds.
    pub fn width(&self, default: f32) -> f32 {
        self.preferred.unwrap_or(default)
    }
    /// Inspect whether the user has replaced the responsive default.
    pub fn preferred(&self) -> Option<f32> {
        self.preferred
    }
    /// Explicit pane-layout commands validate the common width bound and end a previous gesture.
    pub fn set_width(&mut self, width: f32) -> Result<(), String> {
        if !width.is_finite() || !(180.0..=640.0).contains(&width) {
            return Err("Column width must be between 180 and 640 logical points".into());
        }
        self.preferred = Some(width);
        self.resize = None;
        Ok(())
    }
    /// Escape, focus loss and compact/floating layout transitions restore the pre-gesture choice.
    pub(super) fn cancel_resize(&mut self) {
        if let Some(resize) = self.resize.take() {
            self.preferred = resize.before;
        }
    }
    /// Pointer and keyboard adjustments move the named edge; a held stationary pointer does not
    /// keep repainting. Leading edges invert width changes while retaining physical arrow direction.
    pub(super) fn resize(&mut self, ui: &mut Ui, id: Id, default: f32, sash: Sash<'_>) -> f32 {
        let max = sash.max.max(sash.min);
        if !ui.is_enabled()
            || ui.input(|input| {
                input.key_pressed(egui::Key::Escape) || input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false)))
            })
        {
            self.cancel_resize();
            return self.width(default).clamp(sash.min, max);
        }
        let width = self.width(default).clamp(sash.min, max);
        let response = solidcraft_ui_kit::panel_host_sash(ui, id, sash.bounds, sash.name, false, width);
        let sign = if sash.leading { -1.0 } else { 1.0 };
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if response.drag_started()
            && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
        {
            self.resize = Some(Resize { pointer: pointer.x, start: width, before: self.preferred });
        }
        let mut next = None;
        if response.dragged()
            && let Some(resize) = &self.resize
            && let Some(pointer) = response.interact_pointer_pos()
        {
            next = Some(resize.start + (pointer.x - resize.pointer) * sign);
        }
        if response.has_focus() {
            let delta = ui.input_mut(|input| {
                (input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) as i8
                    - input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) as i8) as f32
                    * Tokens::get().space_16
            });
            if delta != 0.0 {
                next = Some(width + delta * sign);
            }
        }
        if let Some(next) = next {
            let next = next.clamp(sash.min, max);
            if self.preferred != Some(next) {
                self.preferred = Some(next);
                ui.ctx().request_repaint();
            }
        }
        if response.drag_stopped() {
            self.resize = None;
        }
        self.width(default).clamp(sash.min, max)
    }
}
