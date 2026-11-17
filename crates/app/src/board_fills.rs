//! View-owned fill scheduling and display of the engine's immutable analytic result.
use crate::{
    board_drc::jobs::{Queue, Request},
    canvas_camera::Camera,
};
use pcbcraft_engine::{
    Session,
    board::{
        fill::FillJob,
        view::{BoardTarget, BoardView},
        work::Source,
    },
    schematic::Point,
};
use std::collections::BTreeSet;

/// One owner per board schedules work; all views read the shared source-keyed engine slot.
#[derive(Default)]
pub(super) struct State {
    queue: Queue<FillJob>,
    requested: Option<Source>,
    generation: u64,
    pub(super) notice: Option<String>,
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
        if let Some(completion) = self.queue.poll()
            && completion.id == self.generation
            && Some(completion.source.target()) == target
            && completion.source.is_current(session)
        {
            // Canvas painting precedes this poll. Completion needs one more frame even when
            // the queue becomes idle and authored revision did not change.
            ctx.request_repaint();
            self.notice = completion.report.err();
            if !completion.retryable
                && let Some(error) = &self.notice
            {
                session.fail_zone_fill(&completion.source, error.clone());
            }
            if completion.retryable {
                self.requested = None;
            }
        }
        self.queue.busy() || (zones && target.is_some() && self.requested.is_none())
    }
}
impl crate::PcbApp {
    /// Remove pours through the shared engine command and immediately retire every view's
    /// in-flight work for that board. Other layers, zone outlines and authored history stay intact.
    pub(crate) fn queue_zone_unfill(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let result = self.session.execute(pcbcraft_engine::UNFILL_ZONES, params).map_err(|e| e.to_string())?;
        let target = self
            .session
            .project()
            .ok_or("No project is open.")?
            .resolve_board(params.get("board").and_then(serde_json::Value::as_str))
            .map_err(|e| e.to_string())?;
        for view in self.views.all_mut().filter(|view| view.board.target == Some(target)) {
            view.board.fills.cancel();
        }
        Ok(result)
    }

    /// Remove all derived pours on the focused board, retaining selectable authored outlines.
    pub(super) fn unfill_zones(&mut self) {
        if let Some(target) = self.views.active.board.target {
            self.message = self.queue_zone_unfill(&serde_json::json!({"board":target.board})).err();
        }
    }

    /// Live and GUI refill share engine validation/preparation while the view owns scheduling.
    pub(crate) fn queue_zone_refill(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let target = self
            .session
            .project()
            .ok_or("No project is open.")?
            .resolve_board(params.get("board").and_then(serde_json::Value::as_str))
            .map_err(|e| e.to_string())?;
        let view = self.views.all_mut().find(|v| v.board.target == Some(target)).ok_or("Open the addressed board before refilling its zones.")?;
        let job = self.session.prepare_zone_refill(params).map_err(|e| e.to_string())?;
        view.board.fills.submit_job(job);
        self.session.zone_fill_status(target).map_err(|e| e.to_string())
    }
    /// Replace only derived fill; native files, undo and project dirty state remain unchanged.
    pub(super) fn refill_zones(&mut self) {
        if let Some(target) = self.views.active.board.target {
            self.message = self.queue_zone_refill(&serde_json::json!({"board":target.board})).err();
        }
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
) {
    let Some(Ok(fill)) = view.filled_zones() else { return };
    let layers = view.layers();
    for zone in &fill.zones {
        if hidden.contains(&zone.layer.to_string()) {
            continue;
        }
        let Some(layer) = layers.iter().find(|l| l.id == zone.layer) else { continue };
        let color = appearance.color(view, layer).gamma_multiply(0.32);
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
            assert_eq!(app.views.view(peer).unwrap().board.fills.queue.completed, 0, "focus changes must not duplicate current-source work");
        }
    }
}
