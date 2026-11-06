//! Two schematic views over one session. Focus moves whole view state, never document state.

use egui::{Rect, Ui};
use pcbcraft_chrome::{editor, grid, shell, workspace};
use pcbcraft_engine::{Session, selection::Selection};

use crate::{Dialog, PcbApp, SchematicView, Tool, ViewId, ViewInput, preferences::SelectionFollow, schematic_canvas};

/// The two views retain their identities when hidden or focused. Opening a split seeds the
/// other camera from the active view; subsequent navigation is independent. Only the
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
    pending_reveal: Option<PendingReveal>,
}

/// A single navigation request tied to both live view addresses and the policy at selection.
/// It is consumed once, after the target host has supplied its current usable canvas area.
#[derive(Debug)]
struct PendingReveal {
    source: ViewInput,
    target: ViewInput,
    policy: SelectionFollow,
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
            pending_reveal: None,
        }
    }
}

impl EditorViews {
    /// Give both views the same snap spacing, as at startup from the remembered preference.
    /// An out-of-range spacing leaves both unchanged.
    pub(super) fn set_grid_everywhere(&mut self, spacing_nm: i64) {
        self.active.camera.set_snap_nm(spacing_nm);
        self.other.camera.set_snap_nm(spacing_nm);
    }

    /// Synchronize both views, including the hidden one, after every project change.
    pub(super) fn sync(&mut self, session: &Session) {
        let active_changed = self.active.sync(session);
        let other_changed = self.other.sync(session);
        if active_changed || other_changed {
            self.selection_source = None;
            self.pending_reveal = None;
        }
    }

    /// Record a deliberate selection or clear in the active view. The receiving view keeps
    /// its edit selection; rendering the projection never becomes another selection intent.
    fn select(&mut self, selection: Selection, session: &Session, policy: SelectionFollow) {
        self.active.selection = selection;
        self.selection_source = Some(self.active.id());
        self.pending_reveal = None;
        if self.split
            && !self.active.selection.is_empty()
            && policy.allows(self.active.id() == ViewId(0))
            && self.active.shares_sheet(&self.other, session)
            && let (Some(source), Some(target)) = (self.active.input(session), self.other.input(session))
        {
            self.pending_reveal = Some(PendingReveal { source, target, policy });
        }
    }

    /// Consume navigation only for the receiving view, using its current host geometry. Focus,
    /// revisions, rebinding, or preference changes invalidate delayed requests; painting a
    /// highlight never produces another request or repeatedly overrides manual navigation.
    fn reveal_for(&mut self, id: ViewId, session: &Session, policy: SelectionFollow) {
        if id != self.other.id() {
            return;
        }
        if let Some(pending) = self.pending_reveal.take()
            && self.split
            && pending.policy == policy
            && self.active.accepts(pending.source, session)
            && self.other.accepts(pending.target, session)
            && self.active.shares_sheet(&self.other, session)
            && let Some(project) = session.project()
        {
            self.other.camera.reveal_selection(project, &self.active.selection);
        }
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
        self.pending_reveal = None;
        self.active.cancel_gesture();
        self.other.cancel_gesture();
    }

    /// Suspend editor input while Home replaces the panes. Returning to the workspace retains
    /// navigation and selection, but cannot revive events captured before it was hidden.
    pub(super) fn suspend(&mut self) {
        self.pending_reveal = None;
        self.active.release_input();
        self.other.release_input();
    }

    /// Change focus only to the visible other view. Invalidate old input before exchanging the
    /// slots, so leaving and returning to a view cannot revive an earlier gesture.
    fn activate(&mut self, id: ViewId) -> bool {
        if !self.split || id != self.other.id() {
            return false;
        }
        self.pending_reveal = None;
        self.active.release_input();
        self.other.release_input();
        std::mem::swap(&mut self.active, &mut self.other);
        true
    }

    /// Open the other view at the active camera, preserving both edit selections. Hiding keeps
    /// the active view. Layout transitions invalidate gestures and pending reveals without
    /// touching the session; reopening deliberately seeds navigation again.
    fn set_split(&mut self, split: bool) {
        if self.split != split {
            self.pending_reveal = None;
            self.active.release_input();
            self.other.release_input();
            if split {
                self.other.camera = self.active.camera.fork_navigation();
            }
            self.split = split;
            self.rects = [(ViewId(0), Rect::NOTHING), (ViewId(1), Rect::NOTHING)];
        }
    }
}

impl PcbApp {
    /// Route explicit selection through the coordinator with the current follow policy.
    pub(super) fn select(&mut self, selection: Selection) {
        self.views.active.selected_leg = None;
        self.views.active.whole_wire_drag = false;
        self.views.select(selection, &self.session, self.preferences.selection_follow);
    }

    /// Select one revision-bound segment while projecting its wire identity to other views.
    pub(super) fn select_leg(&mut self, leg: schematic_canvas::LegTarget) {
        self.select(Selection::one_wire(leg.wire.clone()));
        self.views.active.selected_leg = Some(leg);
    }

    /// Change one view's snap spacing and remember it for the next start. This is presentation
    /// only: the project, its revision, its unsaved state and undo history stay as they are,
    /// nothing moves, and the other view keeps its own spacing. An in-progress gesture is left
    /// alone; its next snapped point follows the new spacing. Out-of-range values are ignored.
    pub(super) fn set_view_grid(&mut self, id: ViewId, spacing_nm: i64) {
        let view = if self.views.active.id() == id { &mut self.views.active } else { &mut self.views.other };
        if view.camera.set_snap_nm(spacing_nm) {
            self.preferences.schematic_grid_nm = spacing_nm;
        }
    }

    /// Step the active view to the next finer (`finer`) or coarser grid preset, wrapping.
    pub(super) fn cycle_grid(&mut self, finer: bool) {
        let current = self.views.active.camera.snap_nm();
        if let Some(next) = grid::cycle(current, &grid::SCHEMATIC_SPACING_PRESETS_NM, finer) {
            self.set_view_grid(self.views.active.id(), next);
        }
    }

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

    /// Render split panes with sheet tabs in stable view order, or a single canvas with an
    /// overlaid breadcrumb and no tab clearance. Workspace panels address the focused view;
    /// their geometry changes with the layout, never with focus alone.
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
                if self.views.split
                    && let Some(path) = view.sheet_path(&self.session)
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
                    self.views.other.camera.set_uncovered(canvas, uncovered);
                    // A context-menu action in the earlier pane can open a modal this frame.
                    if matches!(self.dialog, Dialog::None) {
                        self.views.reveal_for(id, &self.session, self.preferences.selection_follow);
                    }
                    let highlight = self.views.highlight_for(id, &self.session);
                    let view = &mut self.views.other;
                    ui.push_id(view.widget_id(), |ui| {
                        shell::canvas(ui, |ui| {
                            if let Some(project) = self.session.project() {
                                schematic_canvas::show(
                                    ui,
                                    project,
                                    self.session.revision(),
                                    &mut view.camera,
                                    None,
                                    &view.selection,
                                    view.selected_leg.as_ref(),
                                    view.whole_wire_drag,
                                    &highlight,
                                    false,
                                    self.preferences.navigation,
                                    self.preferences.grid_opacity_percent,
                                    &view.wire_draft,
                                    if id == ViewId(0) { "Schematic canvas 1" } else { "Schematic canvas 2" },
                                );
                            }
                        });
                    });
                }
                if !self.views.split
                    && let Some(path) = self.views.active.sheet_path(&self.session)
                {
                    workspace::sheet_heading(ui.painter(), canvas, self.browser.rect(), &path, "1 / 1");
                }
            });
        }
        if self.views.pending_reveal.is_some() {
            // A selection in the later-painted pane must wake the earlier target host.
            ui.ctx().request_repaint();
        }
    }

    /// Draw shared panels once against the workspace bounds, reserving tab clearance only
    /// while split. Return a Browser pick with its originating view address so centering
    /// can use that pane's actual uncovered canvas.
    fn editor_panels(&mut self, ui: &mut Ui) -> Option<(crate::ViewInput, crate::project_browser::Pick)> {
        let source = self.views.active.input(&self.session)?;
        #[cfg(not(target_arch = "wasm32"))]
        let clearance = self.assistant.rail_clearance();
        #[cfg(target_arch = "wasm32")]
        let clearance = 0.0;
        self.net_inspector.show(ui, &self.session, clearance);
        let mut area = ui.available_rect_before_wrap();
        if self.views.split {
            area.min.y = (area.min.y + editor::tab_height() + ui.spacing().item_spacing.y).min(area.max.y);
        }
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
