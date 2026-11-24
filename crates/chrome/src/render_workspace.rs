//! Shared presentation for the board render workspace.
use egui::{Context, Pos2, Rect, Ui};
use solidcraft_ui_kit::view_compass::{self, Compass, ViewAxes};
use std::hash::Hash;

use crate::workspace::{TreeIcon, TreeRow, tree_row};

pub use view_compass::{Drag, ViewAction};

/// Draw and handle the shared compass, keeping its dragging separate from canvas input.
pub fn compass(ui: &mut Ui, rect: Rect, axes: [[f64; 3]; 3], drag: &mut Drag) -> (Option<ViewAction>, Option<view_compass::OrbitStep>) {
    if !ui.is_enabled() {
        *drag = Drag::default();
    }
    let centre = rect.center();
    let may_start = ui.is_enabled() && ui.rect_contains_pointer(rect);
    let orbit = view_compass::orbit(ui, drag, may_start, |p| rect.contains(p));
    let [right, up, back] = axes;
    let action = view_compass::show(
        ui,
        &Compass {
            clip: rect,
            centre,
            axes: ViewAxes { right, up, back },
            hover: may_start.then(|| ui.input(|input| input.pointer.hover_pos())).flatten(),
            orbiting: orbit.orbiting,
            snap_hint: false,
            home_label: "Isometric view".into(),
        },
    );
    (action, orbit.step)
}

/// Draw the normal Browser's board glyphs and rows inside the shared form surface.
pub fn browse_boards<K: Copy + Eq + Hash + std::fmt::Debug>(
    ctx: &Context,
    boards: &[(K, &str)],
    selected: &mut Option<K>,
) -> crate::catalog::FormAction {
    let can_open = boards.iter().any(|(id, _)| Some(*id) == *selected);
    crate::catalog::property_form_enabled(ctx, "browse_render_board", "Browse Boards", 360.0, "Open", can_open, |ui| {
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            if boards.is_empty() {
                ui.label("This project has no PCB.");
            }
            for &(id, name) in boards {
                let row = TreeRow {
                    depth: 0,
                    fold: None,
                    icon: Some(TreeIcon::Board),
                    label: name,
                    accessibility_label: Some(name),
                    tip: "",
                    selected: Some(id) == *selected,
                };
                if tree_row(ui, ui.make_persistent_id(("render_board", id)), &row).clicked {
                    *selected = Some(id);
                    ctx.request_repaint();
                }
            }
        });
    })
}

/// Place the compass at the upper right, clipped to the pane bounds.
pub fn compass_rect(canvas: Rect) -> Rect {
    let size = 122.0;
    Rect::from_min_size(Pos2::new(canvas.right() - size - 8.0, canvas.top() + 8.0), egui::vec2(size, size)).intersect(canvas)
}
