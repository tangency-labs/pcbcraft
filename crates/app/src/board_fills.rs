//! View-owned fill scheduling and display of the engine's immutable analytic result.
use crate::{
    Dialog,
    board_drc::jobs::{Queue, Request},
    canvas_camera::Camera,
    editor_layout::HostId,
    editor_view::ViewId,
};
use pcbcraft_engine::{
    CommandError, Session,
    board::{
        fill::FillJob,
        view::{BoardTarget, BoardView},
        work::Source,
    },
    schematic::Point,
};
use std::collections::BTreeSet;

/// A modal belongs to the exact manual request and the view that submitted it.
#[derive(Clone, Copy, Debug)]
pub(super) struct PourDialog {
    view: ViewId,
    target: BoardTarget,
    generation: u64,
    started_at: Option<f64>,
    completed: usize,
    total: usize,
}

/// One owner per board schedules work; all views read the shared source-keyed engine slot.
#[derive(Default)]
pub(super) struct State {
    queue: Queue<FillJob>,
    requested: Option<Source>,
    generation: u64,
    pub(super) notice: Option<String>,
    #[cfg(test)]
    pub(super) pause_queue: bool,
}
impl std::fmt::Debug for State {
    /// Keep snapshots and derived meshes out of routine view-state logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoneFills").field("queue", &self.queue).field("generation", &self.generation).finish()
    }
}
impl State {
    /// Revoke this owner's current and pending work after explicit removal, without joining.
    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.requested = None;
        self.notice = None;
        self.queue.cancel();
    }

    /// Queue a previously validated shared command without starting engine work in the UI.
    fn submit_job(&mut self, job: FillJob) {
        self.requested = Some(job.source().clone());
        self.generation = self.generation.wrapping_add(1);
        self.notice = None;
        self.queue.submit(Request { id: self.generation, job });
    }
    /// Show active work or a current calculation failure in the existing board status surface.
    pub(super) fn status(&self) -> Option<&str> {
        if self.requested.is_none() {
            None
        } else if self.queue.busy() {
            Some("Filling copper zones")
        } else {
            self.notice.as_deref()
        }
    }
    /// Capture through the same engine preparation API used by explicit/headless refill.
    fn submit(&mut self, session: &Session, target: BoardTarget, refresh: bool) -> Result<(), String> {
        let job = session.prepare_zone_fill(target, refresh).map_err(|e| e.to_string())?;
        self.submit_job(job);
        Ok(())
    }

    /// Revoke obsolete work immediately and expose only a complete result with current source
    /// authority. Capacity failures may retry; geometry failures wait for an edit or explicit refill.
    fn poll(&mut self, session: &Session, target: Option<BoardTarget>, zones: bool, schedule: bool, ctx: &egui::Context) -> bool {
        let obsolete = self.requested.as_ref().is_some_and(|source| Some(source.target()) != target || !source.is_current(session));
        if (!zones || target.is_none() || obsolete) && self.requested.take().is_some() {
            self.queue.cancel();
        }
        if schedule
            && zones
            && self.requested.is_none()
            && let Some(target) = target
        {
            self.notice = self.submit(session, target, false).err();
        }
        #[cfg(test)]
        if self.pause_queue {
            return self.queue.busy();
        }
        if let Some(completion) = self.queue.poll()
            && completion.id == self.generation
            && Some(completion.source.target()) == target
            && completion.source.is_current(session)
        {
            // Canvas painting precedes this poll. Completion needs one more frame even when
            // the queue becomes idle and authored revision did not change.
            ctx.request_repaint();
            let retryable = completion.retryable();
            self.notice = completion.report.err();
            if !retryable && let Some(error) = &self.notice {
                session.fail_zone_fill(&completion.source, error.clone());
            }
            if retryable {
                self.requested = None;
            }
        }
        self.queue.busy() || (schedule && zones && target.is_some() && self.requested.is_none())
    }
}
impl crate::PcbApp {
    /// Remove pours through the shared engine command and immediately retire every view's
    /// in-flight work for that board. Other layers, zone outlines and authored history stay intact.
    pub(crate) fn queue_zone_unfill(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let result = self.session.execute(pcbcraft_engine::UNFILL_ZONES, params)?;
        let target = self
            .session
            .project()
            .ok_or_else(|| CommandError::Unavailable("No project is open.".into()))?
            .resolve_board(params.get("board").and_then(serde_json::Value::as_str))?;
        for view in self.views.all_mut().filter(|view| view.board.target == Some(target)) {
            view.board.fills.cancel();
        }
        Ok(result)
    }

    /// Remove all derived pours on the focused board, retaining selectable authored outlines.
    pub(super) fn unfill_zones(&mut self) {
        if let Some(target) = self.views.active.board.target {
            self.message = self.queue_zone_unfill(&serde_json::json!({"board":target.board})).err().map(|error| error.to_string());
        }
    }

    /// Live and GUI refill share engine validation/preparation while the view owns scheduling.
    fn submit_zone_refill(&mut self, params: &serde_json::Value) -> Result<PourDialog, CommandError> {
        let target = self
            .session
            .project()
            .ok_or_else(|| CommandError::Unavailable("No project is open.".into()))?
            .resolve_board(params.get("board").and_then(serde_json::Value::as_str))?;
        let view = self
            .views
            .all_mut()
            .find(|v| v.board.target == Some(target))
            .ok_or_else(|| CommandError::Unavailable("Open the addressed board before refilling its zones.".into()))?;
        let job = self.session.prepare_zone_refill(params)?;
        let total = job.progress().total;
        view.board.fills.submit_job(job);
        Ok(PourDialog { view: view.id(), target, generation: view.board.fills.generation, started_at: None, completed: 0, total })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn queue_zone_refill(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let pour = self.submit_zone_refill(params)?;
        self.session.zone_fill_status(pour.target)
    }
    /// Replace only derived fill; native files, undo and project dirty state remain unchanged.
    pub(super) fn refill_zones(&mut self) {
        if let Some(target) = self.views.active.board.target {
            self.message = match self.submit_zone_refill(&serde_json::json!({"board":target.board})) {
                Ok(pour) => {
                    self.dialog = Dialog::CopperPour(pour);
                    None
                }
                Err(error) => Some(error.to_string()),
            };
        }
    }

    pub(super) fn copper_pour_host(&self) -> Option<HostId> {
        let Dialog::CopperPour(pour) = self.dialog else { return None };
        self.views.owner(pour.view)
    }

    /// Show only this request's progress and keep elapsed time advancing during kernel work.
    pub(super) fn copper_pour_dialog(&mut self, ctx: &egui::Context, mut pour: PourDialog) {
        let running = self.views.view(pour.view).map(|view| &view.board.fills).filter(|state| {
            state.generation == pour.generation
                && state.queue.busy()
                && state.requested.as_ref().is_some_and(|source| source.target() == pour.target && source.is_current(&self.session))
        });
        let Some(running) = running else { return };
        if let Some(progress) = running.queue.progress_for(pour.generation) {
            pour.completed = progress.completed;
        }
        let now = ctx.input(|input| input.time);
        let elapsed = (now - *pour.started_at.get_or_insert(now)).max(0.0) as u64;
        let fraction = pour.completed as f32 / pour.total.max(1) as f32;
        let mut cancel = false;
        let dismissed = pcbcraft_chrome::shell::dialog(ctx, "copper_pour", "Pouring copper", |ui| {
            ui.label(format!("{} of {} zones · {}:{:02} elapsed", pour.completed, pour.total, elapsed / 60, elapsed % 60));
            pcbcraft_chrome::controls::progress(ui, "copper_pour", fraction);
            cancel = pcbcraft_chrome::controls::button(ui, "Cancel").clicked();
        });
        if cancel || dismissed {
            self.cancel_copper_pour(pour.target);
            self.repaint_editors(ctx);
        } else {
            self.dialog = Dialog::CopperPour(pour);
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }

    /// Revoke the current fill slot as well as the queued job. Otherwise a late worker
    /// or the automatic scheduler could publish a pour after the dialog has dismissed.
    pub(super) fn cancel_copper_pour(&mut self, target: BoardTarget) {
        match self.queue_zone_unfill(&serde_json::json!({"board":target.board})) {
            Ok(_) => self.file_notice = Some("Copper pour canceled".into()),
            Err(error) => self.message = Some(error.to_string()),
        }
    }

    /// Automatic repours leave the canvas usable. Offer the same cancellation from
    /// the status bar even when a peer pane owns the work.
    pub(super) fn cancelable_pour(&mut self) -> Option<BoardTarget> {
        let target = self.views.active.board.target?;
        self.views
            .all_mut()
            .any(|view| {
                let fills = &view.board.fills;
                fills.queue.busy() && fills.requested.as_ref().is_some_and(|source| source.target() == target && source.is_current(&self.session))
            })
            .then_some(target)
    }

    /// Automatic refill follows committed source changes, not pointer previews. Per-board
    /// deduplication limits new submissions; every view still drains its owned queue.
    pub(super) fn poll_zone_fills(&mut self, ctx: &egui::Context) {
        let mut owners = std::collections::BTreeMap::new();
        for view in self.views.all_mut() {
            if let Some(target) = view.board.target
                && view.board.fills.requested.as_ref().is_some_and(|source| source.target() == target && source.is_current(&self.session))
            {
                owners.entry(target).or_insert(view.id());
            }
        }
        for view in self.views.all_mut() {
            let target = view.board.target;
            let schedule = target.is_some_and(|t| *owners.entry(t).or_insert(view.id()) == view.id());
            let zones = target.and_then(|t| self.session.project()?.board_view(t)).is_some_and(|v| !v.zones().is_empty() && v.zone_fill_enabled());
            if view.board.fills.poll(&self.session, target, zones, schedule, ctx) {
                ctx.request_repaint_after(std::time::Duration::from_millis(if view.board.fills.queue.busy() { 8 } else { 250 }));
            }
        }
    }
}

/// Paint only a ready fill of this exact source. Mesher triangles carry no electrical
/// authority and hidden copper layers do not leak fill into the visible canvas.
pub(super) fn paint(
    p: &egui::Painter,
    rect: egui::Rect,
    camera: &Camera,
    view: BoardView<'_>,
    hidden: &BTreeSet<String>,
    appearance: &crate::board_appearance::Appearance,
    palette: Option<&std::collections::BTreeMap<uuid::Uuid, egui::Color32>>,
) {
    let Some(Ok(fill)) = view.filled_zones() else { return };
    let layers = view.layers();
    for zone in &fill.zones {
        if hidden.contains(&zone.layer.to_string()) {
            continue;
        }
        let Some(layer) = layers.iter().find(|l| l.id == zone.layer) else { continue };
        let color = palette.and_then(|palette| palette.get(&zone.net)).copied().unwrap_or_else(|| appearance.color(view, layer)).gamma_multiply(0.32);
        for island in &zone.islands {
            let mut mesh = egui::Mesh::default();
            mesh.vertices = island
                .mesh
                .vertices_nm
                .iter()
                .map(|at| egui::epaint::Vertex {
                    pos: camera.screen(Point { x_nm: at[0].round() as i64, y_nm: at[1].round() as i64 }, rect),
                    uv: egui::epaint::WHITE_UV,
                    color,
                })
                .collect();
            mesh.indices = island.mesh.triangles.iter().flatten().copied().collect();
            p.add(egui::Shape::mesh(mesh));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// Changing which pane is first must not strand the previous owner's ready result or failure.
    #[test]
    fn switching_active_view_drains_every_owned_fill_queue() {
        for failure in [false, true] {
            let mut h = crate::tests::fill_harness();
            let ctx = egui::Context::default();
            let app = h.state_mut();
            let original = app.views.active.id();
            app.split_editors(&ctx, true);
            let peer = app.views.all_mut().find(|v| v.id() != original).unwrap().id();
            let target = app.views.view(original).unwrap().board.target.unwrap();
            app.views.view_mut(peer).unwrap().board.target = Some(target);
            if failure {
                let drawings: Vec<_> = app.session.project().unwrap().board_view(target).unwrap().drawings().iter().map(|d| d.id).collect();
                for drawing in drawings {
                    app.session
                        .apply(pcbcraft_engine::DELETE_DRAWING, &json!({"drawing":drawing,"expected_revision":app.session.revision()}))
                        .unwrap();
                }
            }
            let job = app.session.prepare_zone_fill(target, true).unwrap();
            app.views.view_mut(original).unwrap().board.fills.submit_job(job);
            app.views.bind(peer);
            let before = app.views.view(original).unwrap().board.fills.queue.completed;
            for _ in 0..32 {
                app.poll_zone_fills(&ctx);
                if !app.views.view(original).unwrap().board.fills.queue.busy() {
                    break;
                }
            }
            let old = &app.views.view(original).unwrap().board.fills;
            assert!(!old.queue.busy(), "non-first owner remains pending");
            assert!(old.queue.completed > before);
            assert_eq!(old.notice.is_some(), failure);
            let peer_fills = &mut app.views.view_mut(peer).unwrap().board.fills;
            assert_eq!(peer_fills.queue.completed, 0, "focus changes must not duplicate current-source work");
            assert!(!peer_fills.poll(&app.session, Some(target), true, false, &ctx), "an idle peer must not request another frame");
        }
    }
}
