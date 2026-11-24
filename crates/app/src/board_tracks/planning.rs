//! View-owned walkaround requests. Painting may retain old valid geometry, never its authority.
use super::*;
use crate::board_drc::jobs::{Queue, Request as Queued};
use pcbcraft_engine::{
    Session,
    board::route::{Outcome, Plan, PlanningContext, PlanningJob, Posture, Request},
};

/// A click fixes one target; pointer motion cannot replace it before its plan is adopted.
#[derive(Clone, Debug)]
pub(super) enum Action {
    Vertex { finish: bool },
    Via(String),
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    source: ViewInput,
    board: BoardTarget,
    request: Request,
    prefix: Vec<Point>,
    changes: Vec<(usize, String)>,
    sizes: (i64, i64, i64),
}
/// A fully planned and validated local step, ready for adoption by its owning gesture.
#[derive(Debug)]
pub(super) struct Adoption {
    pub points: Vec<Point>,
    pub replacements: Vec<Replacement>,
    pub via_moves: Vec<ViaMove>,
    pub action: Action,
}
struct Wanted {
    key: Key,
    generation: u64,
    action: Option<Action>,
    /// Explicit finish arriving while the captured vertex is still validating.
    finish_requested: bool,
}
#[derive(Clone)]
struct Ready {
    key: Key,
    plan: Plan,
}
impl Ready {
    /// Extend the fixed prefix with this plan, omitting its duplicate route head.
    fn points(&self) -> Vec<Point> {
        let mut points = self.key.prefix.clone();
        points.extend(self.plan.points.iter().copied().skip(1));
        points
    }
}

/// Retain one coalesced search, reusable obstacle preparation and the latest valid preview.
#[derive(Default)]
pub(super) struct Planning {
    context: Option<PlanningContext>,
    /// Input owner whose context was checked by the current frame before any inline work.
    inline_source: Option<ViewInput>,
    queue: Queue<PlanningJob>,
    wanted: Option<Wanted>,
    /// Latest completion owns placement eligibility, even when it contains no path.
    ready: Option<Ready>,
    /// Last drawable legal path survives a blocked/exhausted replacement under the same source.
    visible: Option<Ready>,
    generation: u64,
    /// Generation whose displayed geometry was submitted for full command preparation.
    prepared_generation: Option<u64>,
    submitted: bool,
    error: Option<String>,
    retryable: bool,
}
impl std::fmt::Debug for Planning {
    /// Show lifecycle occupancy without formatting captured project data.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutePlanning").field("queue", &self.queue).finish()
    }
}
impl Planning {
    /// Cancel adoption immediately, independently of when the worker acknowledges cancellation.
    pub(super) fn cancel(&mut self) {
        self.queue.cancel();
        self.inline_source = None;
        self.wanted = None;
        self.ready = None;
        self.visible = None;
        self.prepared_generation = None;
        self.submitted = false;
        self.error = None;
        self.retryable = false;
        self.generation = self.generation.wrapping_add(1);
    }
    /// Freeze a clicked target until its matching plan completes, or coalesce pointer previews.
    pub(super) fn request(&mut self, draft: &Draft, board: BoardTarget, at: Point, flipped: bool, sizes: (i64, i64, i64), action: Option<Action>) {
        if self.committing() {
            return;
        }
        let (Ok(layer), Ok(start_layer)) = (Uuid::parse_str(&draft.layer), Uuid::parse_str(&draft.start)) else {
            self.cancel();
            return;
        };
        let head = draft.points.last().copied().unwrap_or(at);
        let request = Request {
            mode: draft.mode,
            merge_same_net: draft.merge_same_net,
            fixed: if draft.mode != Mode::Walkaround {
                let mut pieces: Vec<_> = draft
                    .points
                    .windows(2)
                    .zip(segment_layers(&draft.start, &draft.changes, draft.points.len()))
                    .filter_map(|(p, layer)| {
                        Uuid::parse_str(layer).ok().map(|layer| pcbcraft_engine::board::route::FixedCopper::Segment {
                            start: p[0],
                            end: p[1],
                            layer,
                            width_nm: sizes.0,
                        })
                    })
                    .collect();
                pieces.extend(
                    draft
                        .changes
                        .iter()
                        .map(|(at, _)| pcbcraft_engine::board::route::FixedCopper::Via { at: draft.points[*at], diameter_nm: sizes.1 }),
                );
                pieces
            } else {
                Vec::new()
            },
            head,
            target: at,
            layer,
            origin: draft.points.first().copied().map(|point| (point, start_layer)),
            width_nm: sizes.0,
            posture: Posture::Diagonal,
            flipped,
            previous: Vec::new(),
        };
        let mut key = Key { source: draft.source, board, request, prefix: draft.points.clone(), changes: draft.changes.clone(), sizes };
        // Hints are semantic continuity, not another input generation. Reuse them only from
        // this exact head/layer/size/posture and source, never across a vertex or via.
        if let Some(ready) = &self.visible
            && ready.key.source == key.source
            && ready.key.prefix == key.prefix
            && ready.key.changes == key.changes
            && ready.key.sizes == key.sizes
            && ready.key.request.flipped == flipped
            && ready.key.request.layer == layer
        {
            key.request.previous = ready.plan.points.clone();
        }
        if let Some(wanted) = &mut self.wanted {
            // The previous hint may change after a completion, with no actual pointer change.
            let hint = std::mem::replace(&mut key.request.previous, wanted.key.request.previous.clone());
            if wanted.key == key {
                if action.is_some() && self.retryable {
                    self.error = None;
                    self.retryable = false;
                    self.submitted = false;
                }
                wanted.action = action.or(wanted.action.take());
                return;
            }
            key.request.previous = hint;
        }
        self.queue.cancel();
        self.generation = self.generation.wrapping_add(1);
        self.wanted = Some(Wanted { key, generation: self.generation, action, finish_requested: false });
        self.prepared_generation = None;
        self.submitted = false;
        self.error = None;
        self.retryable = false;
        // Cold geometry stays on the worker. A warm query may advance a maximum of eight
        // suspension units here; unfinished search state is handed off without replay.
        if let (Some(context), Some(wanted)) = (&self.context, &self.wanted)
            && self.inline_source == Some(wanted.key.source)
            && context.source().target() == wanted.key.board
            && context.source().revision() == wanted.key.source.revision
        {
            match context.try_prepare_cached(wanted.key.request.clone(), wanted.key.request.mode.default_probe_budget()) {
                Ok(Some(mut job)) => {
                    self.submitted = true;
                    if job.step(8) {
                        let key = wanted.key.clone();
                        self.accept(key, job.take_result().map_err(|e| e.to_string()));
                    } else {
                        self.queue.submit(Queued { id: wanted.generation, job });
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.error = Some(error.to_string());
                    self.submitted = true;
                }
            }
        }
    }
    /// A final click owns its target; Escape and view invalidation still cancel it.
    pub(super) fn committing(&self) -> bool {
        self.wanted.as_ref().is_some_and(|w| w.action.is_some())
    }
    /// Retain Enter/double-click finishing while an already captured vertex is still pending.
    /// Via actions keep their separate layer-transition intent.
    pub(super) fn finish_pending_vertex(&mut self) {
        if let Some(wanted) = &mut self.wanted
            && matches!(wanted.action, Some(Action::Vertex { .. }))
        {
            wanted.finish_requested = true;
        }
    }
    /// Whether the displayed path resolves the current target or its explicit legal adjustment.
    /// Adjusted endpoints can fix ordinary vertices; they cannot authorize a via at the cursor.
    pub(super) fn current_ready(&self) -> bool {
        matches!((&self.wanted, &self.ready), (Some(w), Some(r)) if w.key == r.key
            && (r.plan.outcome == Outcome::Reached || r.plan.adjusted_target.is_some() && !matches!(w.action.as_ref(), Some(Action::Via(_)))))
    }
    /// Bind validation to the current planning generation after its exact geometry is submitted.
    pub(super) fn mark_preparation_requested(&mut self) {
        if self.current_ready() {
            self.prepared_generation = self.wanted.as_ref().map(|wanted| wanted.generation);
        }
    }
    /// Older prepared geometry cannot validate a newly solved pointer or latched click.
    pub(super) fn validation_current(&self) -> bool {
        self.current_ready() && self.wanted.as_ref().is_some_and(|wanted| self.prepared_generation == Some(wanted.generation))
    }
    /// Distinguish an invalid pickup from a legal start whose current target is obstructed.
    /// Only a current terminal result can revoke the initial click, never a stale preview.
    pub(super) fn start_blocked(&self) -> bool {
        matches!((&self.wanted,&self.ready),(Some(w),Some(r)) if w.key==r.key && r.plan.start_blocked && matches!(r.plan.outcome,Outcome::Blocked {..}))
    }
    /// Exact points a current reached request will adopt, including a one-point plan at
    /// the fixed head. Retained display geometry never supplies validation authority.
    pub(super) fn current_path(&self) -> Option<Vec<Point>> {
        self.ready.as_ref().filter(|_| self.current_ready()).map(Ready::points)
    }
    /// Validate the same replacements the current result will adopt, including a
    /// reached plan at the fixed head whose old visible preview is still on screen.
    pub(super) fn current_plan(&self) -> Option<&Plan> {
        self.ready.as_ref().filter(|_| self.current_ready()).map(|ready| &ready.plan)
    }

    /// Status distinguishes a solved route from blocked/exhausted work without certifying DRC.
    pub(super) fn status(&self) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(error.clone());
        }
        let wanted = self.wanted.as_ref()?;
        Some(
            match self.ready.as_ref().filter(|r| r.key == wanted.key).map(|r| r.plan.outcome) {
                Some(Outcome::Reached) => "Route planned",
                Some(Outcome::Blocked { .. }) => "Route blocked",
                Some(Outcome::Incomplete) => "Route search incomplete",
                Some(Outcome::Canceled | Outcome::Stale) => "Route canceled",
                None => "Planning route",
            }
            .into(),
        )
    }
    /// Retain displayed geometry under its captured source and fixed prefix.
    pub(super) fn preview(&self, draft: &Draft, width: i64) -> Vec<Point> {
        let mut path = draft.points.clone();
        if let Some(plan) = self.displayed(draft, width) {
            path.extend(plan.points.iter().copied().skip(1));
        }
        path
    }

    /// A superseded result may still draw a path while replanning but cannot flag old blockers.
    pub(super) fn collisions(&self) -> Option<&BTreeSet<String>> {
        let ready = self.ready.as_ref()?;
        self.wanted.as_ref().filter(|wanted| wanted.key == ready.key && !ready.plan.collisions.is_empty())?;
        Some(&ready.plan.collisions)
    }
    /// Return displacement geometry only for the displayed route's source, prefix and sizes.
    /// Retained geometry cannot authorize placement without current full-path validation.
    fn displayed(&self, draft: &Draft, width: i64) -> Option<&Plan> {
        self.visible
            .as_ref()
            .filter(|ready| {
                ready.key.source == draft.source
                    && ready.key.prefix == draft.points
                    && ready.key.changes == draft.changes
                    && ready.key.request.width_nm == width
                    && ready.key.request.layer.to_string() == draft.layer
                    && ready.key.request.mode == draft.mode
                    && ready.key.request.merge_same_net == draft.merge_same_net
            })
            .map(|ready| &ready.plan)
    }
    /// Track and via previews share the same source and gesture eligibility.
    pub(super) fn displacements<'a>(&'a self, draft: &'a Draft, width: i64) -> (&'a [Replacement], &'a [ViaMove]) {
        self.displayed(draft, width).map_or((&draft.replacements, &draft.via_moves), |plan| (&plan.replacements, &plan.via_moves))
    }
    /// Consume only completed read-only results before paint, preserving input cancellation
    /// priority over placement. No candidate is adopted and no project mutation occurs here.
    pub(super) fn poll_preview(&mut self, session: &Session, source: Option<ViewInput>, layer: Option<&str>, sizes: (i64, i64, i64)) {
        if self
            .wanted
            .as_ref()
            .is_some_and(|w| Some(w.key.source) != source || Some(w.key.request.layer.to_string()).as_deref() != layer || w.key.sizes != sizes)
            || self
                .context
                .as_ref()
                .is_some_and(|c| !c.source().is_current(session) || self.wanted.as_ref().is_some_and(|w| w.key.board != c.source().target()))
        {
            self.cancel();
            self.context = None;
        }
        if let Some(completion) = self.queue.poll()
            && let Some(wanted) = &mut self.wanted
            && wanted.generation == completion.id
            && completion.source.is_current(session)
        {
            let key = wanted.key.clone();
            self.retryable = completion.retryable();
            self.accept(key, completion.report);
        }
        self.inline_source = self.context.as_ref().filter(|context| context.source().is_current(session)).and(source);
    }
    /// Keep the current verdict distinct from display-only fallback geometry on every path.
    fn accept(&mut self, key: Key, result: Result<Option<Plan>, String>) {
        match result {
            Ok(Some(plan)) => {
                let ready = Ready { key, plan };
                let draws_leg = ready.plan.points.len() >= 2 && matches!(ready.plan.outcome, Outcome::Reached | Outcome::Blocked { .. });
                let clear_highlight = ready.key.request.mode == Mode::Highlight && ready.plan.outcome == Outcome::Reached;
                if draws_leg || clear_highlight {
                    self.visible = Some(ready.clone());
                }
                self.ready = Some(ready);
            }
            Ok(None) => self.error = Some("Route canceled".into()),
            Err(error) => self.error = Some(error),
        }
    }
    /// Poll without waiting; reject stale source/layer/size before consuming any completion.
    pub(super) fn poll(
        &mut self,
        session: &Session,
        source: Option<ViewInput>,
        layer: Option<&str>,
        sizes: (i64, i64, i64),
        validation: Option<Result<(), String>>,
    ) -> Option<Result<Adoption, String>> {
        self.poll_preview(session, source, layer, sizes);
        let validation = validation.filter(|_| self.validation_current());
        let wanted = self.wanted.as_mut()?;
        if let Some(error) = &self.error {
            return wanted.action.take().map(|_| Err(error.clone()));
        }
        if let Some(ready) = &self.ready
            && ready.key == wanted.key
        {
            wanted.action.as_ref()?;
            let adjusted = ready.plan.adjusted_target.is_some() && matches!(wanted.action.as_ref(), Some(Action::Vertex { .. }));
            if ready.plan.outcome != Outcome::Reached && !adjusted {
                wanted.action = None;
                return Some(Err(self.status().unwrap_or_default()));
            }
            let points = ready.points();
            if points.len() > MAX_PATH_POINTS {
                wanted.action = None;
                return Some(Err("Route has too many vertices".into()));
            }
            if points.len() >= 2 {
                match validation {
                    None => return None,
                    Some(Err(error)) => {
                        wanted.action = None;
                        return Some(Err(error));
                    }
                    Some(Ok(())) => {}
                }
            }
            let mut action = wanted.action.take()?;
            if adjusted {
                // The actual endpoint is clear of the contacted object, so this fixes a bend
                // without automatically terminating on copper beneath the blocked cursor.
                action = Action::Vertex { finish: wanted.finish_requested };
            } else if wanted.finish_requested {
                action = Action::Vertex { finish: true };
            }
            self.wanted = None;
            return Some(Ok(Adoption { points, replacements: ready.plan.replacements.clone(), via_moves: ready.plan.via_moves.clone(), action }));
        }
        if !self.submitted && !self.queue.busy() {
            let result = (|| {
                if self.context.is_none() {
                    self.context = Some(session.route_planning(wanted.key.board).map_err(|e| e.to_string())?);
                    self.inline_source = Some(wanted.key.source);
                }
                let context = self.context.as_ref().ok_or("Route source unavailable")?;
                session
                    .prepare_route_plan(context, wanted.key.request.clone(), wanted.key.request.mode.default_probe_budget())
                    .map_err(|e| e.to_string())
            })();
            match result {
                Ok(job) => {
                    self.queue.submit(Queued { id: wanted.generation, job });
                    self.submitted = true;
                }
                Err(error) => self.error = Some(error),
            }
        }
        None
    }
    /// The pending via is validated together with its path before the layer transition.
    pub(super) fn via_layer(&self) -> Option<&str> {
        match self.wanted.as_ref()?.action.as_ref()? {
            Action::Via(layer) => Some(layer),
            _ => None,
        }
    }
    /// Request frames while searching or retiring a superseded worker, never after an error.
    pub(super) fn busy(&self) -> bool {
        self.queue.busy() || (self.wanted.is_some() && !self.submitted && self.error.is_none())
    }
    /// Whether an incremental or background planning job is currently queued or running.
    #[cfg(all(test, feature = "performance-harness"))]
    pub(super) fn queue_busy(&self) -> bool {
        self.queue.busy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_view::{EditorView, ViewId, Workspace};

    /// A command-created two-layer board and a revision-bound view, with a free route head.
    fn fixture() -> (Session, EditorView, Draft) {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board":true})).unwrap();
        s.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
        let mut view = EditorView::new(ViewId(10));
        view.bind_opened_project(&s);
        view.switch(Workspace::Board, &s);
        let layer = s
            .project()
            .unwrap()
            .board_view(view.board.target.unwrap())
            .unwrap()
            .layers()
            .into_iter()
            .find(|l| l.purpose == LayerPurpose::Copper)
            .unwrap()
            .id
            .to_string();
        view.board.active_layer = Some(layer.clone());
        let draft = Draft {
            source: view.input(&s).unwrap(),
            mode: Default::default(),
            shove_prefix_dirty: false,
            merge_same_net: false,
            via_moves: Vec::new(),
            replacements: Vec::new(),
            contact: None,
            start: layer.clone(),
            layer,
            points: vec![Point { x_nm: 0, y_nm: 0 }],
            changes: Vec::new(),
            clearance_nm: 200_000,
            via_rules: None,
        };
        (s, view, draft)
    }

    /// The checkbox revokes old previews, follows the request/commit, and cannot alter fixed vertices.
    #[test]
    fn same_net_merge_policy_is_captured_and_locked_with_the_gesture() {
        let (_, view, mut draft) = fixture();
        draft.mode = Mode::Shove;
        let mut state = State { draft: Some(draft), ..Default::default() };
        assert!(!state.merge_same_net);
        let sizes = (250_000, 600_000, 300_000);
        let at = Point { x_nm: 5_000_000, y_nm: 0 };
        state.planning.request(state.draft.as_ref().unwrap(), view.board.target.unwrap(), at, false, sizes, None);
        assert!(!state.planning.wanted.as_ref().unwrap().key.request.merge_same_net);
        assert!(state.select_same_net_merge(true));
        assert!(state.planning.wanted.is_none());
        state.planning.request(state.draft.as_ref().unwrap(), view.board.target.unwrap(), at, false, sizes, None);
        assert!(state.planning.wanted.as_ref().unwrap().key.request.merge_same_net);
        state.draft.as_mut().unwrap().points.push(at);
        assert!(!state.select_same_net_merge(false));
        assert!(state.draft.as_ref().unwrap().merge_same_net);
        let draft = state.draft.take().unwrap();
        let command = add(&mut state, draft).unwrap();
        assert_eq!(command.params["merge_same_net"], true);
    }

    /// Replacements keep only the latest target; a click freezes it even before preparation.
    #[test]
    fn coalesced_search_freezes_the_clicked_target_and_preserves_old_valid_preview() {
        let (s, view, draft) = fixture();
        let mut p = Planning::default();
        let sizes = (250_000, 600_000, 300_000);
        let a = Point { x_nm: 1_000_000, y_nm: 0 };
        let b = Point { x_nm: 5_000_000, y_nm: 0 };
        p.request(&draft, view.board.target.unwrap(), a, false, sizes, None);
        for _ in 0..100 {
            p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Ok(())));
            if p.current_ready() {
                break;
            }
        }
        assert!(p.current_ready());
        p.mark_preparation_requested();
        assert!(p.validation_current());
        assert_eq!(p.preview(&draft, sizes.0).last(), Some(&a));
        // Exercise the deferred branch even when this clear target could finish inline.
        p.inline_source = None;
        p.request(&draft, view.board.target.unwrap(), b, false, sizes, Some(Action::Vertex { finish: true }));
        assert!(!p.current_ready());
        assert!(!p.validation_current());
        assert_eq!(p.preview(&draft, sizes.0).last(), Some(&a));
        p.request(&draft, view.board.target.unwrap(), Point { x_nm: 9_000_000, y_nm: 0 }, false, sizes, None);
        let mut result = None;
        for _ in 0..100 {
            p.mark_preparation_requested();
            if let Some(done) = p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Ok(()))) {
                result = Some(done);
                break;
            }
        }
        let Adoption { points, action, .. } = result.unwrap().unwrap();
        assert_eq!(points.last(), Some(&b));
        assert!(matches!(action, Action::Vertex { finish: true }));
        assert_eq!(s.revision(), draft.source.revision);
    }

    /// Cancel, view release, layer/size changes, shared edits and reopen revoke final clicks.
    #[test]
    fn canceled_and_obsolete_searches_never_adopt_their_click() {
        for (mode, warm) in (0..6).flat_map(|mode| [(mode, false), (mode, true)]) {
            let (mut s, mut view, draft) = fixture();
            let mut p = Planning::default();
            let sizes = (250_000, 600_000, 300_000);
            if warm {
                p.request(&draft, view.board.target.unwrap(), Point { x_nm: 1_000_000, y_nm: 0 }, false, sizes, None);
                for _ in 0..100 {
                    p.poll(&s, view.input(&s), Some(&draft.layer), sizes, None);
                    if p.current_ready() {
                        break;
                    }
                }
                assert!(p.current_ready());
            }
            p.request(&draft, view.board.target.unwrap(), Point { x_nm: 5_000_000, y_nm: 0 }, false, sizes, Some(Action::Vertex { finish: true }));
            // Warm planning may finish inline, but still cannot adopt before validation.
            assert!(p.poll(&s, view.input(&s), Some(&draft.layer), sizes, None).is_none());
            match mode {
                0 => p.cancel(),
                1 => view.release_input(),
                2 => view.board.active_layer = None,
                3 => {}
                4 => {
                    s.apply("project.rename", &json!({"name":"Shared edit"})).unwrap();
                }
                _ => {
                    let snapshot = s.snapshot().unwrap();
                    s.open_snapshot(snapshot);
                }
            }
            for _ in 0..100 {
                p.mark_preparation_requested();
                assert!(
                    p.poll(
                        &s,
                        view.input(&s),
                        view.board.active_layer.as_deref(),
                        if mode == 3 { (500_000, 600_000, 300_000) } else { sizes },
                        Some(Ok(()))
                    )
                    .is_none()
                );
            }
            assert!(!p.committing());
            assert!(s.project().unwrap().board_view(view.board.target.unwrap()).unwrap().tracks().is_empty());
        }
    }

    /// A geometric plan alone cannot fix a vertex; downstream full-path validation owns adoption.
    #[test]
    fn clicked_target_waits_for_full_path_validation_and_reports_its_failure() {
        let (s, view, draft) = fixture();
        let mut p = Planning::default();
        let sizes = (250_000, 600_000, 300_000);
        let target = Point { x_nm: 5_000_000, y_nm: 0 };
        p.request(&draft, view.board.target.unwrap(), target, false, sizes, Some(Action::Vertex { finish: true }));
        for _ in 0..100 {
            assert!(p.poll(&s, view.input(&s), Some(&draft.layer), sizes, None).is_none());
        }
        assert!(p.current_ready());
        assert!(p.committing());
        assert!(p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Ok(()))).is_none(), "unbound old validation cannot fix this target");
        p.mark_preparation_requested();
        let failure = p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Err("Class conflict".into()))).unwrap().unwrap_err();
        assert_eq!(failure, "Class conflict");
        assert!(!p.committing());
        assert!(s.project().unwrap().board_view(view.board.target.unwrap()).unwrap().tracks().is_empty());
    }

    /// An exhausted real engine search cannot erase the last legal ghost or authorize a click.
    #[test]
    fn exhausted_search_preserves_visible_path_without_making_it_current() {
        let (s, view, draft) = fixture();
        let mut p = Planning::default();
        let sizes = (250_000, 600_000, 300_000);
        let good = Point { x_nm: 2_000_000, y_nm: 0 };
        p.request(&draft, view.board.target.unwrap(), good, false, sizes, None);
        for _ in 0..100 {
            p.poll(&s, view.input(&s), Some(&draft.layer), sizes, None);
            if p.current_ready() {
                break;
            }
        }
        assert!(p.current_ready());
        let visible = p.preview(&draft, sizes.0);
        assert_eq!(visible.last(), Some(&good));
        // Retain the old display while the deliberately exhausted replacement is deferred.
        p.inline_source = None;
        p.request(&draft, view.board.target.unwrap(), Point { x_nm: 5_000_000, y_nm: 0 }, false, sizes, None);
        // Use an actual exhausted engine job; only its caller-supplied budget differs.
        let wanted = p.wanted.as_ref().unwrap();
        let job = s.prepare_route_plan(p.context.as_ref().unwrap(), wanted.key.request.clone(), 1).unwrap();
        p.queue.submit(Queued { id: wanted.generation, job });
        p.submitted = true;
        for _ in 0..100 {
            p.poll(&s, view.input(&s), Some(&draft.layer), sizes, None);
            if p.status().as_deref() == Some("Route search incomplete") {
                break;
            }
        }
        assert_eq!(p.status().as_deref(), Some("Route search incomplete"));
        assert!(!p.current_ready());
        assert!(p.current_path().is_none(), "display fallback cannot authorize validation");
        assert_eq!(p.preview(&draft, sizes.0), visible, "a failed search must not erase legal geometry");
        p.request(&draft, view.board.target.unwrap(), Point { x_nm: 5_000_000, y_nm: 0 }, false, sizes, Some(Action::Vertex { finish: true }));
        assert!(p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Ok(()))).unwrap().is_err());
        assert!(s.project().unwrap().board_view(view.board.target.unwrap()).unwrap().tracks().is_empty());
        p.cancel();
        assert_eq!(p.preview(&draft, sizes.0), draft.points);
    }

    /// Returning to the fixed head retains the old ghost but validates and adopts only
    /// the prefix. Pending requests supply no candidate.
    #[test]
    fn one_point_reached_plan_keeps_display_separate_from_adoption() {
        let (s, view, mut draft) = fixture();
        let head = Point { x_nm: 2_000_000, y_nm: 0 };
        draft.points.push(head);
        draft.mode = Mode::Shove;
        let mut p = Planning::default();
        let sizes = (250_000, 600_000, 300_000);
        let target = Point { x_nm: 5_000_000, y_nm: 0 };
        p.request(&draft, view.board.target.unwrap(), target, false, sizes, None);
        assert!(p.current_path().is_none());
        for _ in 0..100 {
            p.poll(&s, view.input(&s), Some(&draft.layer), sizes, None);
            if p.current_ready() {
                break;
            }
        }
        assert_eq!(p.current_path().unwrap().last(), Some(&target));
        let visible = p.preview(&draft, sizes.0);
        p.visible.as_mut().unwrap().plan.replacements.push(Replacement { track: Uuid::from_u128(1), points: vec![head, target] });
        let next = s
            .project()
            .unwrap()
            .board_view(view.board.target.unwrap())
            .unwrap()
            .layers()
            .into_iter()
            .find(|layer| layer.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper && layer.id.to_string() != draft.layer)
            .unwrap()
            .id
            .to_string();
        p.request(&draft, view.board.target.unwrap(), head, false, sizes, Some(Action::Via(next.clone())));
        for _ in 0..100 {
            assert!(p.poll(&s, view.input(&s), Some(&draft.layer), sizes, None).is_none());
            if p.current_ready() {
                break;
            }
        }
        assert!(p.current_ready());
        assert_eq!(p.ready.as_ref().unwrap().plan.points, vec![head]);
        assert_eq!(p.preview(&draft, sizes.0), visible);
        assert_eq!(p.current_path(), Some(draft.points.clone()));
        assert_eq!(p.displacements(&draft, sizes.0).0.len(), 1);
        let validation_replacements = p.current_plan().unwrap().replacements.clone();
        assert!(validation_replacements.is_empty());
        p.mark_preparation_requested();
        let adopted = p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Ok(()))).unwrap().unwrap();
        assert_eq!(adopted.points, draft.points);
        assert_eq!(adopted.replacements, validation_replacements);
        assert!(matches!(adopted.action, Action::Via(layer) if layer == next));
        assert!(p.current_path().is_none());
    }
}
