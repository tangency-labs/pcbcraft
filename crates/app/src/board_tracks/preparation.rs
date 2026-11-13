//! View-owned routing preparation. Only the latest captured input can publish a prepared edit.
use super::*;
use crate::{
    PcbApp,
    board_drc::jobs::{Queue, Request},
};
use pcbcraft_engine::{
    Session,
    board::track::preparation::{CopperEditJob, CopperPreparation, PreparedCopperEdit},
};
use serde_json::Value;

#[derive(Clone, PartialEq)]
struct Key {
    source: ViewInput,
    command: &'static str,
    params: Value,
    layer: Option<String>,
    sizes: (i64, i64, i64),
}
struct Wanted {
    key: Key,
    generation: u64,
    commit: bool,
}
struct Ready {
    key: Key,
    edit: PreparedCopperEdit,
}

/// One running route job, one coalesced request and one ready candidate. Context projection
/// is reused while its immutable source remains current; canceled generations never publish.
#[derive(Default)]
pub(super) struct Preparation {
    context: Option<CopperPreparation>,
    queue: Queue<CopperEditJob>,
    wanted: Option<Wanted>,
    ready: Option<Ready>,
    generation: u64,
    notice: Option<String>,
    failed: bool,
    pub stale: u64,
}
impl std::fmt::Debug for Preparation {
    /// Diagnostics expose lifecycle state, without formatting captured project contents.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutePreparation").field("queue", &self.queue).field("generation", &self.generation).finish()
    }
}
impl Preparation {
    /// Replace only obsolete work. Promoting the same preview to a commit reuses its validation.
    pub(super) fn request(&mut self, edit: Command, layer: Option<String>, sizes: (i64, i64, i64), commit: bool) {
        let key = Key { source: edit.source, command: edit.id, params: edit.params, layer, sizes };
        if let Some(wanted) = &mut self.wanted
            && wanted.key == key
        {
            wanted.commit |= commit;
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.queue.cancel();
        self.ready = self.ready.take().filter(|ready| ready.key == key);
        self.wanted = Some(Wanted { key, generation: self.generation, commit });
        self.notice = None;
        self.failed = false;
    }
    /// Revoke publication immediately; worker cancellation remains cooperative and never joins.
    pub(super) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.wanted = None;
        self.ready = None;
        self.queue.cancel();
        self.notice = None;
        self.failed = false;
    }
    /// A final route awaits authoritative publication, retaining its ghost until completion.
    pub(super) fn committing(&self) -> bool {
        self.wanted.as_ref().is_some_and(|w| w.commit)
    }
    /// Whether the latest pointer's candidate has completed the ordinary command validation.
    pub(super) fn ready(&self) -> bool {
        self.ready.is_some() && self.wanted.is_some()
    }
    /// Current lifecycle information for the status bar, rather than a validity promise for DRC.
    pub(super) fn status(&self) -> Option<String> {
        if let Some(notice) = &self.notice {
            return Some(notice.clone());
        }
        self.wanted.as_ref().map(|_| {
            self.queue
                .progress()
                .map_or_else(|| if self.ready() { "Route prepared".into() } else { "Preparing route".into() }, |p| format!("Route: {}", p.stage))
        })
    }
    /// Poll captured work, canceling changed source/layer/size/view authority before publication.
    /// A complete typed candidate is returned only for the current commit generation.
    fn poll(
        &mut self,
        session: &Session,
        source: Option<ViewInput>,
        target: Option<BoardTarget>,
        layer: Option<&str>,
        sizes: (i64, i64, i64),
    ) -> Option<Result<(ViewInput, PreparedCopperEdit), String>> {
        let obsolete = self.wanted.as_ref().is_some_and(|w| Some(w.key.source) != source || w.key.layer.as_deref() != layer || w.key.sizes != sizes)
            || self.context.as_ref().is_some_and(|c| !c.source().is_current(session));
        if obsolete {
            self.stale += 1;
            self.cancel();
            self.context = None;
        }
        let completion = self.queue.poll();
        if let Some(completion) = completion
            && self.wanted.as_ref().is_some_and(|w| w.generation == completion.id)
        {
            let wanted = self.wanted.as_ref()?;
            if !completion.source.is_current(session) {
                self.stale += 1;
                self.cancel();
            } else {
                match completion.report {
                    Ok(Some(edit)) => self.ready = Some(Ready { key: wanted.key.clone(), edit }),
                    Ok(None) => {}
                    Err(error) => {
                        let commit = wanted.commit;
                        self.ready = None;
                        self.failed = true;
                        self.notice = Some(error.clone());
                        if commit {
                            self.wanted = None;
                            return Some(Err(error));
                        }
                    }
                }
            }
        }
        let wanted = self.wanted.as_ref()?;
        if self.failed {
            if wanted.commit {
                self.wanted = None;
                return Some(Err(self.notice.clone().unwrap_or_else(|| "Copper preparation failed".into())));
            }
            return None;
        }
        if let Some(ready) = self.ready.take() {
            if ready.key == wanted.key && wanted.commit {
                let source = wanted.key.source;
                self.wanted = None;
                self.queue.cancel();
                return Some(Ok((source, ready.edit)));
            }
            self.ready = Some(ready);
        }
        if self.ready.is_none() && !self.queue.busy() {
            let result = (|| {
                let target = target.ok_or("board unavailable")?;
                if self.context.is_none() {
                    self.context = Some(session.copper_preparation(target).map_err(|e| e.to_string())?);
                }
                let context = self.context.as_ref().ok_or("route source unavailable")?;
                let wanted = self.wanted.as_ref().ok_or("route request canceled")?;
                let job = session.prepare_copper_edit_with(context, wanted.key.command, &wanted.key.params).map_err(|e| e.to_string())?;
                Ok::<_, String>(Request { id: wanted.generation, job })
            })();
            match result {
                Ok(request) => self.queue.submit(request),
                Err(error) => {
                    let commit = self.committing();
                    self.failed = true;
                    self.notice = Some(error.clone());
                    if commit {
                        self.wanted = None;
                        return Some(Err(error));
                    }
                }
            }
        }
        None
    }
    /// Whether another UI pass is needed for a pending or retiring worker.
    fn busy(&self) -> bool {
        self.queue.busy() || (self.wanted.is_some() && self.ready.is_none() && !self.failed)
    }
}
impl PcbApp {
    /// Route canvas commands through preparation after releasing the immutable painting borrow.
    pub(crate) fn prepare_canvas_copper(&mut self, edit: Command) {
        let tracks = &mut self.views.active.board.tracks;
        let sizes = (tracks.width_nm, tracks.via_diameter_nm, tracks.via_drill_nm);
        tracks.preparation.request(edit, self.views.active.board.active_layer.clone(), sizes, true);
    }
    /// Publish only current view-owned prepared commands through the shared session transaction.
    /// Polling never waits on a worker, and a source change invalidates other views' pending edits.
    pub(crate) fn poll_copper_preparations(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        let blocked = !matches!(self.dialog, crate::Dialog::None) || self.files.busy() || self.project_action_pending() || self.parts.is_modal();
        for view in self.views.all_mut() {
            let source = view.input(&self.session);
            let target = view.board.target;
            let layer = view.board.active_layer.clone();
            let tracks = &mut view.board.tracks;
            if blocked {
                tracks.preparation.cancel();
            }
            let sizes = (tracks.width_nm, tracks.via_diameter_nm, tracks.via_drill_nm);
            let completion = tracks.preparation.poll(&self.session, source, target, layer.as_deref(), sizes);
            if let Some(completion) = completion {
                let result = completion.and_then(|(input, edit)| {
                    if Some(input) != source {
                        return Err("route input changed before publication".into());
                    }
                    self.session.commit_copper_edit(edit).map_err(|e| e.to_string())
                });
                tracks.committed = None;
                tracks.committed_ends.clear();
                tracks.committed_move = None;
                match result {
                    Ok(_) => {
                        tracks.retry_route = None;
                        self.message = None;
                        changed = true;
                    }
                    Err(error) => {
                        if let Some(draft) = tracks.retry_route.take().filter(|d| Some(d.source) == source) {
                            tracks.draft = Some(draft);
                        }
                        self.message = Some(error);
                    }
                }
                ctx.request_repaint();
            }
            if tracks.preparation.busy() {
                ctx.request_repaint();
            }
        }
        if changed {
            self.views.sync(&self.session);
            self.repaint_editors(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_view::{EditorView, ViewId, Workspace};

    /// Build through normal engine commands and bind a view with its own input identity.
    fn fixture() -> (Session, EditorView, String) {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board":true})).unwrap();
        s.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
        let mut view = EditorView::new(ViewId(10));
        view.bind_opened_project(&s);
        view.switch(Workspace::Board, &s);
        let target = s.project().unwrap().board_target().unwrap();
        let layer =
            s.project().unwrap().board_view(target).unwrap().layers().into_iter().find(|l| l.purpose == LayerPurpose::Copper).unwrap().id.to_string();
        view.board.active_layer = Some(layer.clone());
        (s, view, layer)
    }
    /// Make pointer generations distinguishable by their final X coordinate.
    fn command(s: &Session, view: &EditorView, layer: &str, x: i64) -> Command {
        Command::new(
            view.input(s).unwrap(),
            pcbcraft_engine::ADD_TRACKS,
            json!({"board":view.board.target.unwrap().board,"layer":layer,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":0},{"x_nm":x,"y_nm":0}],"expected_revision":s.revision()}),
        )
    }
    /// Poll the deterministic browser scheduler with the same authority used by the live host.
    fn poll(p: &mut Preparation, s: &Session, view: &EditorView, layer: &str) -> Option<Result<(ViewInput, PreparedCopperEdit), String>> {
        p.poll(s, view.input(s), view.board.target, Some(layer), (250_000, 600_000, 300_000))
    }
    /// Pointer replacement discards completed but unconsumed older work, reuses the source context,
    /// retains bounded state and promotes only the latest ready preview to one final transaction.
    #[test]
    fn coalesced_preview_and_out_of_order_completion_publish_only_latest() {
        let (mut s, view, layer) = fixture();
        let mut p = Preparation::default();
        p.request(command(&s, &view, &layer, 1_000_000), Some(layer.clone()), (250_000, 600_000, 300_000), false);
        poll(&mut p, &s, &view, &layer);
        // Leave the active request unconsumed while replacing its publication generation.
        // Even a finished worker is obsolete before the next host poll.
        for x in 2..100 {
            p.request(command(&s, &view, &layer, x * 1_000_000), Some(layer.clone()), (250_000, 600_000, 300_000), false);
        }
        assert!(p.ready.is_none());
        for _ in 0..30 {
            assert!(poll(&mut p, &s, &view, &layer).is_none());
        }
        assert!(p.ready());
        let revision = s.revision();
        p.request(command(&s, &view, &layer, 99_000_000), Some(layer.clone()), (250_000, 600_000, 300_000), true);
        let (_, edit) = poll(&mut p, &s, &view, &layer).unwrap().unwrap();
        s.commit_copper_edit(edit).unwrap();
        assert_eq!(s.revision(), revision + 1);
        assert_eq!(s.project().unwrap().board_view(view.board.target.unwrap()).unwrap().tracks()[0].end.x_nm, 99_000_000);
    }
    /// Escape, focus loss, a different view, local width/layer changes and an intervening MCP edit
    /// revoke publication, including work already complete on the worker.
    #[test]
    fn canceled_or_changed_view_authority_never_publishes() {
        for mode in 0..7 {
            let (mut s, mut view, layer) = fixture();
            view.board.tracks.preparation.request(command(&s, &view, &layer, 1_000_000), Some(layer.clone()), (250_000, 600_000, 300_000), true);
            // Take preparation out briefly to exercise the owner API without aliasing the view.
            let mut p = std::mem::take(&mut view.board.tracks.preparation);
            poll(&mut p, &s, &view, &layer);
            view.board.tracks.preparation = p;
            match mode {
                0 => {
                    assert!(view.board.tracks.escape());
                }
                1 => view.release_input(),
                2 => {
                    s.apply("project.rename", &json!({"name":"MCP changed"})).unwrap();
                }
                3 => {
                    let snapshot = s.snapshot().unwrap();
                    s.open_snapshot(snapshot);
                }
                _ => {}
            }
            let mut p = std::mem::take(&mut view.board.tracks.preparation);
            for _ in 0..30 {
                let result = if mode == 4 {
                    p.poll(&s, view.input(&s), view.board.target, Some(&layer), (500_000, 600_000, 300_000))
                } else if mode == 5 {
                    p.poll(&s, view.input(&s), view.board.target, Some("different-layer"), (250_000, 600_000, 300_000))
                } else if mode == 6 {
                    let mut other = EditorView::new(ViewId(11));
                    other.bind_opened_project(&s);
                    other.switch(Workspace::Board, &s);
                    poll(&mut p, &s, &other, &layer)
                } else {
                    poll(&mut p, &s, &view, &layer)
                };
                assert!(result.is_none());
            }
            assert!(!p.busy());
            assert!(s.project().unwrap().board_view(s.project().unwrap().board_target().unwrap()).unwrap().tracks().is_empty());
        }
    }
}
