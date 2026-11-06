//! Two schematic views over one session. Focus moves whole view state, never document state.

use egui::{Rect, Ui};
use pcbcraft_chrome::{editor, shell, workspace};
use pcbcraft_engine::{Session, selection::Selection};

use crate::{Dialog, PcbApp, SchematicView, Tool, ViewId, schematic_canvas};

/// The two views retain their identities and navigation when hidden or focused. Only the
/// active view can issue edits; moving it between slots does not reconstruct either view.
#[derive(Debug)]
pub(super) struct EditorViews {
    pub(super) active: SchematicView,
    pub(super) other: SchematicView,
    pub(super) split: bool,
    ratio: f32,
    rects: [(ViewId, Rect); 2],
    /// Last explicit selection intent. Focus changes alone never replace it.
    selection_source: Option<ViewId>,
}

impl Default for EditorViews {
    fn default() -> Self {
        Self {
            active: SchematicView::new(ViewId(0)),
            other: SchematicView::new(ViewId(1)),
            split: false,
            ratio: 0.5,
            rects: [(ViewId(0), Rect::NOTHING), (ViewId(1), Rect::NOTHING)],
            selection_source: None,
        }
    }
}

impl EditorViews {
    /// Synchronize both views, including the hidden one, after every project change.
    pub(super) fn sync(&mut self, session: &Session) {
        let active_changed = self.active.sync(session);
        let other_changed = self.other.sync(session);
        if active_changed || other_changed {
            self.selection_source = None;
        }
    }

    /// Record a deliberate selection or clear in the active view. The receiving view keeps
    /// its edit selection; rendering the projection never becomes another selection intent.
    pub(super) fn select(&mut self, selection: Selection) {
        self.active.selection = selection;
        self.selection_source = Some(self.active.id());
    }

    /// Project the latest selection into another visible view of the same sheet. Resolve
    /// against live, pruned view state so deletion, undo and reopening cannot retain stale IDs.
    /// Focus and cameras are unaffected, and the source view never receives its own projection.
    pub(super) fn highlight_for(&self, id: ViewId, session: &Session) -> Selection {
        if !self.split {
            return Selection::default();
        }
        let (source, target) = if self.selection_source == Some(self.active.id()) && id == self.other.id() {
            (&self.active, &self.other)
        } else if self.selection_source == Some(self.other.id()) && id == self.active.id() {
            (&self.other, &self.active)
        } else {
            return Selection::default();
        };
        if source.shares_sheet(target, session) { source.selection.clone() } else { Selection::default() }
    }

    /// Cancel previews in every view when a modal or Home takes over input.
    pub(super) fn cancel_gestures(&mut self) {
        self.active.cancel_gesture();
        self.other.cancel_gesture();
    }

    /// Suspend editor input while Home replaces the panes. Returning to the workspace retains
    /// navigation and selection, but cannot revive events captured before it was hidden.
    pub(super) fn suspend(&mut self) {
        self.active.release_input();
        self.other.release_input();
    }

    /// Change focus only to the visible other view. Invalidate old input before exchanging the
    /// slots, so leaving and returning to a view cannot revive an earlier gesture.
    fn activate(&mut self, id: ViewId) -> bool {
        if !self.split || id != self.other.id() {
            return false;
        }
        self.active.release_input();
        self.other.release_input();
        std::mem::swap(&mut self.active, &mut self.other);
        true
    }

    /// Hide or restore the other view without changing either camera or selection. Layout
    /// transitions invalidate gestures and old pane hit regions, but never touch the session.
    fn set_split(&mut self, split: bool) {
        if self.split != split {
            self.active.release_input();
            self.other.release_input();
            self.split = split;
            self.rects = [(ViewId(0), Rect::NOTHING), (ViewId(1), Rect::NOTHING)];
        }
    }
}

impl PcbApp {
    /// Apply the presentation action from the View ribbon. A modal owns input until it closes.
    pub(super) fn split_editors(&mut self, ctx: &egui::Context, split: bool) {
        if self.session.project().is_none() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        self.views.set_split(split);
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }

    /// Route focus without moving cameras or editing the document. Release text search and
    /// browser drag ownership when a pane takes over; dialog input remains with its owner.
    fn activate_editor(&mut self, ctx: &egui::Context, id: ViewId) {
        if !matches!(self.dialog, Dialog::None) || !self.views.activate(id) {
            return;
        }
        self.parts.release_focus(ctx);
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }

    /// Resolve a new pointer press before consuming keyboard input. A held drag crossing the
    /// divider keeps its original owner; popups and modals do not transfer editor focus.
    pub(super) fn editor_pointer_focus(&mut self, ctx: &egui::Context) {
        if self.home || !matches!(self.dialog, Dialog::None) || egui::Popup::is_any_open(ctx) {
            return;
        }
        let pressed = ctx.input(|input| input.pointer.any_pressed().then(|| input.pointer.interact_pos()).flatten());
        if let Some(pointer) = pressed
            && !ctx.layer_id_at(pointer).is_some_and(|layer| layer.order != egui::Order::Background)
            && let Some((id, _)) = self.views.rects.iter().find(|(_, rect)| rect.contains(pointer))
        {
            self.activate_editor(ctx, *id);
        }
    }

    /// Render sheet tabs and canvases in stable view order. Workspace panels stay at their
    /// window positions and address the focused view; their geometry never follows focus.
    pub(super) fn editor_workspace(&mut self, ui: &mut Ui, workspace: Rect) {
        if self.session.project().is_none() {
            self.canvas(ui);
            return;
        }
        let mut pick = self.editor_panels(ui);
        let panes = if self.views.split {
            let panes = editor::split(ui, &mut self.views.ratio);
            self.views.rects = [(ViewId(0), panes[0]), (ViewId(1), panes[1])];
            self.views.rects.to_vec()
        } else {
            vec![(self.views.active.id(), ui.available_rect_before_wrap())]
        };
        #[cfg(not(target_arch = "wasm32"))]
        let assistant = self.assistant.covered_rect(workspace);
        #[cfg(target_arch = "wasm32")]
        let assistant = {
            let _ = workspace;
            Rect::NOTHING
        };
        let covered = [self.browser.rect(), self.parts.rect(), assistant];
        for (id, rect) in panes {
            ui.scope_builder(egui::UiBuilder::new().id_salt(("editor_pane", id)).max_rect(rect), |ui| {
                ui.set_clip_rect(rect);
                let active = self.views.active.id() == id;
                let view = if active { &self.views.active } else { &self.views.other };
                if let Some(path) = view.sheet_path(&self.session)
                    && editor::sheet_tab(ui, &path, active)
                {
                    self.activate_editor(ui.ctx(), id);
                }
                let canvas = ui.available_rect_before_wrap();
                let uncovered = workspace::catalog::uncovered(canvas, &covered);
                if self.views.active.id() == id {
                    self.views.active.camera.set_uncovered(canvas, uncovered);
                    if let Some((source, pick)) = pick.take() {
                        self.show_pick(source, pick);
                    }
                    self.canvas(ui);
                } else {
                    let highlight = self.views.highlight_for(id, &self.session);
                    let view = &mut self.views.other;
                    view.camera.set_uncovered(canvas, uncovered);
                    ui.push_id(view.widget_id(), |ui| {
                        shell::canvas(ui, |ui| {
                            if let Some(project) = self.session.project() {
                                schematic_canvas::show(
                                    ui,
                                    project,
                                    &mut view.camera,
                                    None,
                                    &view.selection,
                                    &highlight,
                                    false,
                                    self.preferences.navigation,
                                    &view.wire_draft,
                                    if id == ViewId(0) { "Schematic canvas 1" } else { "Schematic canvas 2" },
                                );
                            }
                        });
                    });
                }
            });
        }
    }

    /// Draw shared panels once against the workspace bounds. Return a Browser pick with its
    /// originating view address so centering can use that pane's actual uncovered canvas.
    fn editor_panels(&mut self, ui: &mut Ui) -> Option<(crate::ViewInput, crate::project_browser::Pick)> {
        let source = self.views.active.input(&self.session)?;
        #[cfg(not(target_arch = "wasm32"))]
        let clearance = self.assistant.rail_clearance();
        #[cfg(target_arch = "wasm32")]
        let clearance = 0.0;
        self.net_inspector.show(ui, &self.session, clearance);
        let mut area = ui.available_rect_before_wrap();
        area.min.y = (area.min.y + editor::tab_height() + ui.spacing().item_spacing.y).min(area.max.y);
        let pick = self.browser.show(ui, self.parts.browser_area(area), &self.session, &self.views.active.selection);
        let placing = match &self.views.active.tool {
            Some(Tool::PlaceSymbol { definition, .. }) => Some(definition.clone()),
            _ => None,
        };
        if let Some(definition) = self.parts.show(ui, area, self.browser.rect(), placing.as_ref(), source) {
            self.begin_placement(definition);
        }
        pick.map(|pick| (source, pick))
    }
}
