//! Two schematic views over one session. Focus moves whole view state, never document state.

use egui::{Rect, Ui};
use pcbcraft_chrome::{editor, shell, workspace};
use pcbcraft_engine::Session;

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
}

impl Default for EditorViews {
    fn default() -> Self {
        Self {
            active: SchematicView::new(ViewId(0)),
            other: SchematicView::new(ViewId(1)),
            split: false,
            ratio: 0.5,
            rects: [(ViewId(0), Rect::NOTHING), (ViewId(1), Rect::NOTHING)],
        }
    }
}

impl EditorViews {
    /// Synchronize both views, including the hidden one, after every project change.
    pub(super) fn sync(&mut self, session: &Session) {
        self.active.sync(session);
        self.other.sync(session);
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

    /// Render one or two panes in stable ID order. Inactive panes draw the same snapshot but
    /// cannot consume editing input. A document edit requests another frame for all panes.
    pub(super) fn editor_workspace(&mut self, ui: &mut Ui, workspace: Rect) {
        if !self.views.split || self.session.project().is_none() {
            self.active_editor(ui, workspace);
            return;
        }
        let panes = editor::split(ui, &mut self.views.ratio);
        self.views.rects = [(ViewId(0), panes[0]), (ViewId(1), panes[1])];
        for (index, rect) in panes.into_iter().enumerate() {
            let id = ViewId(index as u64);
            ui.scope_builder(egui::UiBuilder::new().id_salt(("editor_pane", id)).max_rect(rect), |ui| {
                ui.set_clip_rect(rect);
                let active = self.views.active.id() == id;
                if editor::header(ui, if index == 0 { "Schematic 1" } else { "Schematic 2" }, active) {
                    self.activate_editor(ui.ctx(), id);
                }
                if self.views.active.id() == id {
                    self.active_editor(ui, workspace);
                } else {
                    let view = &mut self.views.other;
                    let canvas = ui.available_rect_before_wrap();
                    view.camera.set_uncovered(canvas, canvas);
                    ui.push_id(view.widget_id(), |ui| {
                        shell::canvas(ui, |ui| {
                            if let Some(project) = self.session.project() {
                                schematic_canvas::show(
                                    ui,
                                    project,
                                    &mut view.camera,
                                    None,
                                    &view.selection,
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
}

impl PcbApp {
    /// Draw the contextual panels and canvas for the active view, preserving the single-view
    /// layout. Panels follow focus; their actions carry this view’s input address.
    fn active_editor(&mut self, ui: &mut Ui, _workspace: Rect) {
        if let Some(source) = self.views.active.input(&self.session) {
            #[cfg(not(target_arch = "wasm32"))]
            let clearance = self.assistant.rail_clearance();
            #[cfg(target_arch = "wasm32")]
            let clearance = 0.0;
            self.net_inspector.show(ui, &self.session, clearance);
            let canvas = ui.available_rect_before_wrap();
            let browser_area = self.parts.browser_area(canvas);
            let pick = self.browser.show(ui, browser_area, &self.session, &self.views.active.selection);
            let placing = match &self.views.active.tool {
                Some(Tool::PlaceSymbol { definition, .. }) => Some(definition.clone()),
                _ => None,
            };
            if let Some(definition) = self.parts.show(ui, canvas, self.browser.rect(), placing.as_ref(), source) {
                self.begin_placement(definition);
            }
            #[cfg(not(target_arch = "wasm32"))]
            let assistant = self.assistant.covered_rect(_workspace);
            #[cfg(target_arch = "wasm32")]
            let assistant = egui::Rect::NOTHING;
            self.views
                .active
                .camera
                .set_uncovered(canvas, workspace::catalog::uncovered(canvas, &[self.browser.rect(), self.parts.rect(), assistant]));
            if let Some(pick) = pick {
                self.show_pick(source, pick);
            }
        }
        let canvas = ui.available_rect_before_wrap();
        self.canvas(ui);
        if let Some(project) = self.session.project() {
            // Beside the floating Browser, as in PCBcraft Next; one sheet so far.
            let left = canvas.left() + self.browser.covered_width() + 12.0;
            workspace::sheet_heading(ui.painter(), canvas, left, &format!("{}  /  Main", project.name), "1 / 1");
        }
    }
}
