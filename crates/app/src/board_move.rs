//! View-local mixed board pickup; geometry authority and publication belong to the engine.
use crate::{
    Dialog, PcbApp, ViewId,
    editor_view::{BoardState, ViewInput},
};
use egui::{Context, Ui};
use pcbcraft_engine::{
    Project,
    board::{
        clipboard::BoardSelection,
        movement::{MOVE_ITEMS, MovePlan},
    },
    schematic::Point,
};
use std::{collections::BTreeSet, sync::Arc};

/// One group owns its input until placed or cancelled; a cancellation frame swallows release.
#[derive(Debug, Default)]
pub(super) struct State {
    draft: Option<Draft>,
    cancelled: bool,
}
#[derive(Debug)]
struct Draft {
    source: ViewInput,
    pickup: bool,
    plan: MovePlan,
    items: BoardSelection,
    anchor: Point,
    delta: Point,
    shown: Arc<Project>,
    error: Option<String>,
    hidden: BTreeSet<String>,
    layer: Option<String>,
    filters: crate::selection_filters::Filters,
}
impl State {
    /// Pending cancellation also owns the current release, preventing a new single-item gesture.
    pub(super) fn active(&self) -> bool {
        self.draft.is_some() || self.cancelled
    }
    /// Discard preview geometry while preserving selection and the source project.
    pub(super) fn cancel(&mut self) {
        self.cancelled |= self.draft.take().is_some();
    }
    /// Escape leaves the complete selection intact for another pickup.
    pub(super) fn escape(&mut self) -> bool {
        let had = self.active();
        self.cancel();
        had
    }
}
/// A captured mixed selection must remain completely visible and pickable, never reduced to a subset.
fn subset(a: &BoardSelection, b: &BoardSelection) -> bool {
    a.footprints.is_subset(&b.footprints)
        && a.drawings.is_subset(&b.drawings)
        && a.zones.is_subset(&b.zones)
        && a.tracks.is_subset(&b.tracks)
        && a.vias.is_subset(&b.vias)
}
impl PcbApp {
    /// Start M for any explicit group before category-specific handlers can consume it.
    /// Existing gestures, modal/file input, changed authority, filters or layers cancel the whole group.
    pub(super) fn board_move_ready(&mut self, ctx: &Context, id: ViewId, enabled: bool) -> bool {
        let Some(project) = self.session.snapshot() else { return false };
        let Some(view) = self.views.view_mut(id) else { return false };
        if view.board.movement.active() {
            let stale = view.board.movement.draft.as_ref().is_some_and(|draft| {
                view.input(&self.session) != Some(draft.source)
                    || draft.hidden != view.board.hidden_layers
                    || draft.layer != view.board.active_layer
                    || draft.filters != view.filters
                    || draft.items != crate::board_clipboard::selection(&view.board, &project)
            });
            if !enabled || stale || self.files.busy() || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))) {
                view.board.movement.cancel();
            }
            view.board.move_requested = false;
            return true;
        }
        if !enabled || !view.board.move_requested || view.board.reference.selected.is_some() {
            return false;
        }
        let items = crate::board_clipboard::selection(&view.board, &project);
        if items.len() < 2 {
            return false;
        }
        // Copper-only groups already have a bounded movement handler; leave its route lifecycle intact.
        if items.footprints.is_empty() && items.drawings.is_empty() && items.zones.is_empty() {
            return false;
        }
        view.board.move_requested = false;
        if crate::board_clipboard::busy(&view.board) || !matches!(self.dialog, Dialog::None) || self.files.busy() {
            return false;
        }
        let (Some(source), Some(target)) = (view.input(&self.session), view.board.target) else { return false };
        let Some(board) = project.board_view(target) else { return false };
        let visible = crate::board_canvas::all_visible(board, &view.board, view.filters, &self.board_appearance);
        if !subset(&items, &visible) {
            return false;
        }
        let fallback = board
            .footprints()
            .find(|f| items.footprints.contains(&f.id))
            .map(|f| f.position)
            .or_else(|| board.zones().iter().find(|z| items.zones.contains(&z.id)).and_then(|z| z.points.first().copied()))
            .or_else(|| board.drawings().iter().find(|d| items.drawings.contains(&d.id)).and_then(|d| d.points.first().copied()))
            .or_else(|| board.tracks().iter().find(|t| items.tracks.contains(&t.id)).map(|t| t.start))
            .or_else(|| board.vias().iter().find(|v| items.vias.contains(&v.id)).map(|v| v.at))
            .unwrap_or(Point { x_nm: 0, y_nm: 0 });
        // The exact canvas rectangle is supplied on the first preview pass, before any displacement.
        match MovePlan::new(project.clone(), target, items.clone()) {
            Ok(plan) => {
                view.board.movement.draft = Some(Draft {
                    source,
                    pickup: true,
                    plan,
                    items,
                    anchor: fallback,
                    delta: Point { x_nm: 0, y_nm: 0 },
                    shown: project,
                    error: None,
                    hidden: view.board.hidden_layers.clone(),
                    layer: view.board.active_layer.clone(),
                    filters: view.filters,
                })
            }
            Err(e) => {
                self.message = Some(e.to_string());
                return false;
            }
        }
        true
    }
    /// Paint disposable geometry from the immutable captured source; only a valid placement click
    /// dispatches one revision-checked command. Pointer motion reuses an unchanged-offset preview.
    pub(super) fn board_move_canvas(&mut self, ui: &mut Ui, id: ViewId, label: &str) {
        let Some(original) = self.session.snapshot() else { return };
        let Some(view) = self.views.view_mut(id) else { return };
        let rect = ui.available_rect_before_wrap();
        let mut draft = view.board.movement.draft.take();
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
        if let Some(d) = &mut draft {
            if view.board.move_requested {
                view.board.move_requested = false;
            }
            // First pass: pickup stays under the current pointer without moving the source.
            if d.pickup {
                if let Some(p) = pointer {
                    d.anchor = view.camera.snap(p, rect);
                }
                d.pickup = false;
            }
            if let Some(p) = pointer {
                let at = view.camera.snap(p, rect);
                let delta = Point { x_nm: at.x_nm - d.anchor.x_nm, y_nm: at.y_nm - d.anchor.y_nm };
                if delta != d.delta {
                    d.delta = delta;
                    match d.plan.at(delta) {
                        Ok(shown) => {
                            d.shown = shown;
                            d.error = None;
                        }
                        Err(e) => d.error = Some(e.to_string()),
                    }
                }
            }
        }
        let shown = draft.as_ref().map_or(&original, |d| &d.shown);
        let Some(target) = view.board.target else { return };
        let Some(board) = shown.board_view(target) else { return };
        let items = draft.as_ref().map(|d| d.items.clone()).unwrap_or_else(|| crate::board_clipboard::selection(&view.board, &original));
        let mut state = BoardState {
            target: Some(target),
            active_layer: view.board.active_layer.clone(),
            hidden_layers: view.board.hidden_layers.clone(),
            selection: items.footprints,
            ..Default::default()
        };
        state.drawing.select_many(items.drawings);
        state.zones.zone = true;
        state.zones.select_many(items.zones);
        state.tracks.selected = items.tracks.union(&items.vias).copied().collect();
        let _ = crate::board_canvas::show(
            ui,
            board,
            &mut view.camera,
            &mut state,
            None,
            &Default::default(),
            view.filters,
            &self.board_appearance,
            true,
            self.preferences.navigation,
            self.preferences.grid_opacity(true),
            self.preferences.board_grid_style,
            label,
            &[],
            self.preferences.units,
        );
        view.board.rotate_requested = 0;
        view.board.flip_requested = false;
        if view.board.movement.cancelled {
            view.board.movement.cancelled = ui.input(|i| i.pointer.primary_down());
            return;
        }
        if let Some(d) = draft {
            ui.ctx().set_cursor_icon(if d.error.is_some() { egui::CursorIcon::NotAllowed } else { egui::CursorIcon::Grabbing });
            if ui.input(|i| i.pointer.secondary_clicked()) {
                return;
            }
            if pointer.is_some() && ui.input(|i| i.pointer.primary_clicked()) {
                if let Some(error) = d.error {
                    self.message = Some(error);
                    return;
                }
                if self.views.active.accepts(d.source, &self.session) {
                    self.run(MOVE_ITEMS,&serde_json::json!({"board":target.board,"items":d.items,"dx_nm":d.delta.x_nm,"dy_nm":d.delta.y_nm,"expected_revision":d.source.revision}));
                }
            } else {
                if let Some(view) = self.views.view_mut(id) {
                    view.board.movement.draft = Some(d);
                }
            }
        }
    }
}
