//! View-owned walkaround requests. Painting may retain old valid geometry, never its authority.
use super::*;
use crate::board_drc::jobs::{Queue, Request as Queued};
use pcbcraft_engine::{
    Session,
    board::route::{DEFAULT_BUDGET, Outcome, Plan, PlanningContext, PlanningJob, Posture, Request},
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
struct Wanted {
    key: Key,
    generation: u64,
    action: Option<Action>,
}
#[derive(Clone)]
struct Ready {
    key: Key,
    plan: Plan,
}

/// Retain one coalesced search, reusable obstacle preparation and the latest valid preview.
#[derive(Default)]
pub(super) struct Planning {
    context: Option<PlanningContext>,
    queue: Queue<PlanningJob>,
    wanted: Option<Wanted>,
    /// Latest completion owns placement eligibility, even when it contains no path.
    ready: Option<Ready>,
    /// Last drawable legal path survives a blocked/exhausted replacement under the same source.
    visible: Option<Ready>,
    generation: u64,
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
        self.wanted = None;
        self.ready = None;
        self.visible = None;
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
        let Ok(layer) = Uuid::parse_str(&draft.layer) else {
            self.cancel();
            return;
        };
        let head = draft.points.last().copied().unwrap_or(at);
        let request = Request { head, target: at, layer, width_nm: sizes.0, posture: Posture::Diagonal, flipped, previous: Vec::new() };
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
        self.wanted = Some(Wanted { key, generation: self.generation, action });
        self.submitted = false;
        self.error = None;
        self.retryable = false;
    }
    /// A final click owns its target; Escape and view invalidation still cancel it.
    pub(super) fn committing(&self) -> bool {
        self.wanted.as_ref().is_some_and(|w| w.action.is_some())
    }
    /// Whether the displayed path is also the current pointer's fully solved path.
    pub(super) fn current_ready(&self) -> bool {
        matches!((&self.wanted, &self.ready), (Some(w), Some(r)) if w.key == r.key && r.plan.outcome == Outcome::Reached)
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
    /// Retain the previous legal geometry during longer solves only under its original prefix.
    pub(super) fn preview(&self, draft: &Draft, width: i64) -> Vec<Point> {
        let mut path = draft.points.clone();
        if let Some(ready) = &self.visible
            && ready.key.source == draft.source
            && ready.key.prefix == path
            && ready.key.changes == draft.changes
            && ready.key.request.width_nm == width
            && ready.key.request.layer.to_string() == draft.layer
        {
            path.extend(ready.plan.points.iter().copied().skip(1));
        }
        path
    }
    /// Poll without waiting; reject stale source/layer/size before consuming any completion.
    pub(super) fn poll(
        &mut self,
        session: &Session,
        source: Option<ViewInput>,
        layer: Option<&str>,
        sizes: (i64, i64, i64),
        validation: Option<Result<(), String>>,
    ) -> Option<Result<(Vec<Point>, Action), String>> {
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
            match completion.report {
                Ok(Some(plan)) => {
                    let ready = Ready { key: wanted.key.clone(), plan };
                    if ready.plan.points.len() >= 2 && matches!(ready.plan.outcome, Outcome::Reached | Outcome::Blocked { .. }) {
                        self.visible = Some(ready.clone());
                    }
                    self.ready = Some(ready);
                }
                Ok(None) => self.error = Some("Route canceled".into()),
                Err(error) => {
                    self.error = Some(error);
                    self.retryable = completion.retryable;
                }
            }
        }
        let wanted = self.wanted.as_mut()?;
        if let Some(error) = &self.error {
            return wanted.action.take().map(|_| Err(error.clone()));
        }
        if let Some(ready) = &self.ready
            && ready.key == wanted.key
        {
            wanted.action.as_ref()?;
            if ready.plan.outcome != Outcome::Reached {
                wanted.action = None;
                return Some(Err(self.status().unwrap_or_default()));
            }
            let mut points = wanted.key.prefix.clone();
            points.extend(ready.plan.points.iter().copied().skip(1));
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
            let action = wanted.action.take()?;
            self.wanted = None;
            return Some(Ok((points, action)));
        }
        if !self.submitted && !self.queue.busy() {
            let result = (|| {
                if self.context.is_none() {
                    self.context = Some(session.route_planning(wanted.key.board).map_err(|e| e.to_string())?);
                }
                let context = self.context.as_ref().ok_or("Route source unavailable")?;
                session.prepare_route_plan(context, wanted.key.request.clone(), DEFAULT_BUDGET).map_err(|e| e.to_string())
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
        assert_eq!(p.preview(&draft, sizes.0).last(), Some(&a));
        p.request(&draft, view.board.target.unwrap(), b, false, sizes, Some(Action::Vertex { finish: true }));
        assert!(!p.current_ready());
        assert_eq!(p.preview(&draft, sizes.0).last(), Some(&a));
        p.request(&draft, view.board.target.unwrap(), Point { x_nm: 9_000_000, y_nm: 0 }, false, sizes, None);
        let mut result = None;
        for _ in 0..100 {
            if let Some(done) = p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Ok(()))) {
                result = Some(done);
                break;
            }
        }
        let (points, action) = result.unwrap().unwrap();
        assert_eq!(points.last(), Some(&b));
        assert!(matches!(action, Action::Vertex { finish: true }));
        assert_eq!(s.revision(), draft.source.revision);
    }

    /// Cancel, view release, layer/size changes, shared edits and reopen revoke final clicks.
    #[test]
    fn canceled_and_obsolete_searches_never_adopt_their_click() {
        for mode in 0..6 {
            let (mut s, mut view, draft) = fixture();
            let mut p = Planning::default();
            let sizes = (250_000, 600_000, 300_000);
            p.request(&draft, view.board.target.unwrap(), Point { x_nm: 5_000_000, y_nm: 0 }, false, sizes, Some(Action::Vertex { finish: true }));
            p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Ok(())));
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
        assert_eq!(p.preview(&draft, sizes.0), visible, "a failed search must not erase legal geometry");
        p.request(&draft, view.board.target.unwrap(), Point { x_nm: 5_000_000, y_nm: 0 }, false, sizes, Some(Action::Vertex { finish: true }));
        assert!(p.poll(&s, view.input(&s), Some(&draft.layer), sizes, Some(Ok(()))).unwrap().is_err());
        assert!(s.project().unwrap().board_view(view.board.target.unwrap()).unwrap().tracks().is_empty());
        p.cancel();
        assert_eq!(p.preview(&draft, sizes.0), draft.points);
    }
}
