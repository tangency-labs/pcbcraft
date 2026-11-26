//! View-local mixed board pickup; geometry authority and publication belong to the engine.
use crate::{
    Dialog, PcbApp, ViewId,
    canvas_camera::pointer_release,
    editor_view::{BoardState, ViewInput},
};
use egui::{Context, PointerButton, Ui};
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
    pending_drag: Option<DragPickup>,
}

/// The input, snapped press location and selection captured when the drag starts.
#[derive(Debug)]
struct DragPickup {
    source: ViewInput,
    anchor: Point,
    items: BoardSelection,
}
#[derive(Debug)]
struct Draft {
    source: ViewInput,
    pickup: bool,
    drag: bool,
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
        self.draft.is_some() || self.cancelled || self.pending_drag.is_some()
    }
    /// Discard preview geometry while preserving selection and the source project.
    pub(super) fn cancel(&mut self) {
        let requested = self.pending_drag.take().is_some();
        self.cancelled |= self.draft.take().is_some() || requested;
    }
    /// Keep the press anchor, input and selection until the group takes over on the next pass.
    pub(super) fn request_drag(&mut self, source: ViewInput, anchor: Point, items: BoardSelection) {
        self.pending_drag = Some(DragPickup { source, anchor, items });
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
    /// Start Move or a held drag for an explicit group before category-specific handlers can consume it.
    /// Cancel if input expires or the selection changes before pickup or while moving.
    pub(super) fn board_move_ready(&mut self, ctx: &Context, id: ViewId, enabled: bool) -> bool {
        let Some(project) = self.session.snapshot() else { return false };
        let Some(view) = self.views.view_mut(id) else { return false };
        if !view.board.movement.active() && !view.board.move_requested {
            return false;
        }
        let input = view.input(&self.session);
        let drag = view.board.movement.pending_drag.take();
        let moving = view.board.movement.active();
        if moving || drag.is_some() {
            let stale = drag.as_ref().is_some_and(|drag| input != Some(drag.source))
                || view.board.movement.draft.as_ref().is_some_and(|draft| {
                    input != Some(draft.source)
                        || draft.hidden != view.board.hidden_layers
                        || draft.layer != view.board.active_layer
                        || draft.filters != view.filters
                        || draft.items != crate::board_clipboard::selection(&view.board, &project)
                });
            if !enabled || stale || self.files.busy() || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))) {
                view.board.movement.cancelled |= drag.is_some();
                view.board.movement.cancel();
                view.board.move_requested = false;
                return true;
            }
            if moving {
                view.board.move_requested = false;
                return true;
            }
        }
        if !enabled || view.board.reference.selected.is_some() {
            return false;
        }
        let items = crate::board_clipboard::selection(&view.board, &project);
        if drag.as_ref().is_some_and(|drag| drag.items != items) {
            view.board.movement.cancelled = true;
            view.board.move_requested = false;
            return true;
        }
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
        let (Some(source), Some(target)) = (input, view.board.target) else { return false };
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
        match MovePlan::new(project.clone(), target, items.clone()) {
            Ok(plan) => {
                view.board.movement.draft = Some(Draft {
                    source,
                    pickup: drag.is_none(),
                    drag: drag.is_some(),
                    plan,
                    items,
                    anchor: drag.map_or(fallback, |drag| drag.anchor),
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
    /// Paint disposable geometry from the captured source. A valid click or drag release
    /// dispatches one revision-checked command. A secondary press discards the preview and owns the release.
    pub(super) fn board_move_canvas(&mut self, ui: &mut Ui, id: ViewId, label: &str) {
        let Some(original) = self.session.snapshot() else { return };
        let Some(view) = self.views.view_mut(id) else { return };
        let rect = ui.available_rect_before_wrap();
        let mut draft = view.board.movement.draft.take();
        if draft.is_some() && ui.input(|i| i.pointer.button_pressed(PointerButton::Secondary)) {
            draft = None;
            view.board.movement.cancelled = true;
            view.board.context_menu_allowed = false;
        }
        let release = if draft.as_ref().is_some_and(|d| d.drag) && ui.input(|i| i.pointer.primary_released()) {
            pointer_release(&ui.response(), PointerButton::Primary)
        } else {
            None
        };
        let pointer = release.or_else(|| ui.input(|i| i.pointer.hover_pos())).filter(|p| rect.contains(*p));
        if let Some(d) = &mut draft {
            if view.board.move_requested {
                view.board.move_requested = false;
            }
            // Keyboard pickup takes its anchor from the first paint. Drags already have a press anchor.
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
            true,
            self.preferences.navigation,
            self.preferences.grid_opacity(true),
            self.preferences.board_grid_style,
            label,
            &[],
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
            // A drag places on release over the canvas; a lost release, an off-canvas release or
            // an unchanged offset ends it without an edit.
            let place = if d.drag {
                let released = release.is_some();
                if !released && !ui.input(|i| i.pointer.primary_down()) {
                    return;
                }
                released
            } else {
                ui.input(|i| i.pointer.primary_clicked())
            };
            if place && d.drag && (pointer.is_none() || d.delta == Point { x_nm: 0, y_nm: 0 }) {
                return;
            }
            if pointer.is_some() && place {
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
