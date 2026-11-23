//! Bounded per-view scheduling: one running snapshot and one replaceable pending snapshot.
use pcbcraft_engine::board::drc::{CheckJob, CheckProgress, CheckSource, Report};

/// Engine work shared by checker and routing queues; hosts retain publication authority.
pub(crate) trait Work: Send + 'static {
    type Output: Send + 'static;
    fn source(&self) -> &CheckSource;
    fn progress(&self) -> CheckProgress;
    fn step(&mut self, budget: usize) -> bool;
    fn take_output(&mut self) -> Result<Option<Self::Output>, String>;
    /// Jobs with indivisible kernel work may sample the same host cancellation between calls.
    fn cancellation(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        None
    }
    /// Browser work that cannot safely run in the foreground supplies an isolated request.
    #[cfg(target_arch = "wasm32")]
    fn browser_request(&mut self) -> Option<Result<String, String>> {
        None
    }
    /// Some isolated work prepares a job that must subsequently resume incremental phases.
    #[cfg(target_arch = "wasm32")]
    fn browser_prepares(&self) -> bool {
        false
    }
    /// Decode a completed isolated result into the same engine output as native work.
    #[cfg(target_arch = "wasm32")]
    fn browser_response(&mut self, _response: &str) -> Result<Option<Self::Output>, String> {
        Err("This job has no isolated browser result adapter".into())
    }
}
impl Work for pcbcraft_engine::board::fill::FillJob {
    type Output = std::sync::Arc<pcbcraft_engine::board::fill::Fill>;
    fn source(&self) -> &CheckSource {
        self.source()
    }
    fn progress(&self) -> CheckProgress {
        self.progress()
    }
    fn step(&mut self, budget: usize) -> bool {
        self.step(budget)
    }
    fn take_output(&mut self) -> Result<Option<Self::Output>, String> {
        self.take_result()
    }
    fn cancellation(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        Some(self.cancellation())
    }
    #[cfg(target_arch = "wasm32")]
    fn browser_request(&mut self) -> Option<Result<String, String>> {
        if self.is_ready() { None } else { Some(self.worker_request()) }
    }
    #[cfg(target_arch = "wasm32")]
    fn browser_response(&mut self, response: &str) -> Result<Option<Self::Output>, String> {
        self.accept_worker_response(response)
    }
}
impl Work for CheckJob {
    type Output = Report;
    #[cfg(target_arch = "wasm32")]
    fn browser_request(&mut self) -> Option<Result<String, String>> {
        self.browser_zone_request()
    }
    #[cfg(target_arch = "wasm32")]
    fn browser_prepares(&self) -> bool {
        true
    }
    #[cfg(target_arch = "wasm32")]
    fn browser_response(&mut self, response: &str) -> Result<Option<Report>, String> {
        self.accept_browser_zones(response)?;
        Ok(None)
    }
    fn source(&self) -> &CheckSource {
        self.source()
    }
    fn progress(&self) -> CheckProgress {
        self.progress()
    }
    fn step(&mut self, budget: usize) -> bool {
        self.step(budget)
    }
    fn take_output(&mut self) -> Result<Option<Report>, String> {
        Ok(self.take_report())
    }
}
impl Work for pcbcraft_engine::board::track::preparation::CopperEditJob {
    type Output = Result<pcbcraft_engine::board::track::preparation::PreparedCopperEdit, pcbcraft_engine::CommandError>;
    fn source(&self) -> &CheckSource {
        self.source()
    }
    fn progress(&self) -> CheckProgress {
        self.progress()
    }
    fn step(&mut self, budget: usize) -> bool {
        self.step(budget)
    }
    /// Keep engine refusals in the completed output, distinct from worker failures.
    fn take_output(&mut self) -> Result<Option<Self::Output>, String> {
        Ok(self.take_result().transpose())
    }
}

impl Work for pcbcraft_engine::board::route::PlanningJob {
    type Output = pcbcraft_engine::board::route::Plan;
    fn source(&self) -> &CheckSource {
        self.source()
    }
    fn progress(&self) -> CheckProgress {
        self.progress()
    }
    fn step(&mut self, budget: usize) -> bool {
        self.step(budget)
    }
    fn take_output(&mut self) -> Result<Option<Self::Output>, String> {
        self.take_result().map_err(|e| e.to_string())
    }
}

/// One immutable checker request tagged with its owner's publication generation.
pub(crate) struct Request<J: Work = CheckJob> {
    pub id: u64,
    pub job: J,
}
/// Scheduling can fail before engine validation; only capacity indicates contention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScheduleFailure {
    Capacity,
    #[cfg(not(target_arch = "wasm32"))]
    Spawn,
}

/// A terminal completion carries its source; canceled work contains no report.
pub(crate) struct Completion<T = Report> {
    pub id: u64,
    pub source: CheckSource,
    pub report: Result<Option<T>, String>,
    pub schedule_failure: Option<ScheduleFailure>,
}
impl<T> Completion<T> {
    /// Only scheduling failures may be retried by the owner without changing the command.
    pub fn retryable(&self) -> bool {
        self.schedule_failure.is_some()
    }
}
/// Bounded scheduling retains one active and one replaceable pending request per owner.
pub(crate) struct Queue<J: Work = CheckJob> {
    active: Option<Running<J>>,
    pending: Option<Request<J>>,
    pub replaced: u64,
    pub canceled: u64,
    pub completed: u64,
}
impl<J: Work> Default for Queue<J> {
    fn default() -> Self {
        Self { active: None, pending: None, replaced: 0, canceled: 0, completed: 0 }
    }
}
impl<J: Work> std::fmt::Debug for Queue<J> {
    /// Debug output exposes occupancy without retaining or formatting project contents.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotQueue").field("running", &self.active.is_some()).field("pending", &self.pending.is_some()).finish()
    }
}
impl<J: Work> Queue<J> {
    /// Coalesce replacements without starting another worker until cancellation is acknowledged.
    pub fn submit(&mut self, request: Request<J>) {
        if let Some(active) = &mut self.active {
            active.cancel();
            self.replaced += u64::from(self.pending.is_some());
            self.pending = Some(request);
        } else {
            self.active = Some(Running::start(request));
        }
    }
    /// Signal cancellation and discard queued work without waiting or joining.
    pub fn cancel(&mut self) {
        self.pending = None;
        if let Some(active) = &mut self.active {
            active.cancel();
            self.canceled += 1;
        }
    }
    /// Return at most one completion, then start the latest queued request.
    pub fn poll(&mut self) -> Option<Completion<J::Output>> {
        let result = self.active.as_mut().and_then(Running::poll);
        if result.is_some() {
            self.active = None;
            self.completed += 1;
            if let Some(request) = self.pending.take() {
                self.active = Some(Running::start(request));
            }
        }
        result
    }
    /// Copy stage progress without blocking on a native worker.
    pub fn progress(&self) -> Option<CheckProgress> {
        self.active.as_ref().and_then(Running::progress)
    }
    /// Reject edits/reopen independently of which view currently has focus.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn authority_current(&self, session: &pcbcraft_engine::Session) -> bool {
        self.pending
            .as_ref()
            .map(|r| r.job.source())
            .or_else(|| self.active.as_ref().and_then(Running::source))
            .is_some_and(|s| s.is_current(session))
    }
    /// Only current captured work may continue against the displayed board.
    pub fn current(&self, session: &pcbcraft_engine::Session, target: Option<pcbcraft_engine::board::view::BoardTarget>) -> bool {
        let source = self.pending.as_ref().map(|r| r.job.source()).or_else(|| self.active.as_ref().and_then(Running::source));
        source.is_some_and(|s| Some(s.target()) == target && s.is_current(session))
    }
    /// The retained queue has capacity two, including the executing request.
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }
}
#[cfg(any(target_arch = "wasm32", test))]
use incremental::Running;
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
use native::Running;

#[cfg(any(target_arch = "wasm32", test))]
mod incremental {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen::prelude::wasm_bindgen(inline_js = "export function checker_now() { return performance.now(); }")]
    extern "C" {
        fn checker_now() -> f64;
    }
    pub(super) struct Running<J: Work> {
        request: Option<Request<J>>,
        canceled: bool,
        #[cfg(target_arch = "wasm32")]
        worker: Option<u32>,
        #[cfg(target_arch = "wasm32")]
        failure: Option<String>,
    }
    impl<J: Work> Running<J> {
        /// Retain incremental jobs directly; kernel jobs capture one same-build isolated request.
        pub fn start(request: Request<J>) -> Self {
            #[cfg(target_arch = "wasm32")]
            let mut request = request;
            #[cfg(target_arch = "wasm32")]
            let (worker, failure) = match request.job.browser_request() {
                Some(Ok(payload)) => match super::browser_worker::start(&payload) {
                    Ok(id) => (Some(id), None),
                    Err(error) => (None, Some(error)),
                },
                Some(Err(error)) => (None, Some(error)),
                None => (None, None),
            };
            Self {
                request: Some(request),
                canceled: false,
                #[cfg(target_arch = "wasm32")]
                worker,
                #[cfg(target_arch = "wasm32")]
                failure,
            }
        }
        /// Captured source, absent after an incremental request has been consumed.
        pub fn source(&self) -> Option<&CheckSource> {
            self.request.as_ref().map(|r| r.job.source())
        }
        /// Signal cooperative cancellation without waiting for the current batch.
        pub fn cancel(&mut self) {
            self.canceled = true;
            #[cfg(target_arch = "wasm32")]
            if let Some(worker) = self.worker.take() {
                super::browser_worker::cancel(worker);
            }
            if let Some(cancel) = self.request.as_ref().and_then(|r| r.job.cancellation()) {
                cancel.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        /// Copy the latest progress when available; never block the caller.
        pub fn progress(&self) -> Option<CheckProgress> {
            self.request.as_ref().map(|r| r.job.progress())
        }
        /// Advance one engine batch, publishing only a complete report or cancellation.
        fn poll_batch(&mut self) -> Option<Completion<J::Output>> {
            #[cfg(target_arch = "wasm32")]
            if !self.canceled && (self.worker.is_some() || self.failure.is_some()) {
                let response = if let Some(error) = self.failure.take() {
                    Err(error)
                } else {
                    let id = self.worker?;
                    let response = super::browser_worker::poll(id)?;
                    self.worker = None;
                    response
                };
                let mut request = self.request.take()?;
                let schedule_failure = response.as_ref().err().is_some_and(|e| e.contains("worker capacity")).then_some(ScheduleFailure::Capacity);
                let result = response.and_then(|response| request.job.browser_response(&response));
                if request.job.browser_prepares() && result.is_ok() {
                    self.request = Some(request);
                } else {
                    return Some(Completion { id: request.id, source: request.job.source().clone(), report: result, schedule_failure });
                }
            }
            let request = self.request.as_mut()?;
            if !self.canceled && !request.job.step(256) {
                return None;
            }
            let mut request = self.request.take()?;
            Some(Completion {
                id: request.id,
                source: request.job.source().clone(),
                report: if self.canceled { Ok(None) } else { request.job.take_output() },
                schedule_failure: None,
            })
        }

        /// Yield after two browser milliseconds; tests use one deterministic batch per poll.
        pub fn poll(&mut self) -> Option<Completion<J::Output>> {
            #[cfg(target_arch = "wasm32")]
            {
                let began = checker_now();
                loop {
                    if let Some(completion) = self.poll_batch() {
                        return Some(completion);
                    }
                    if checker_now() - began >= 2.0 {
                        return None;
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            self.poll_batch()
        }
    }
    #[cfg(target_arch = "wasm32")]
    impl<J: Work> Drop for Running<J> {
        /// Closing a view retires only its worker, with no join or foreground kernel work.
        fn drop(&mut self) {
            if let Some(worker) = self.worker.take() {
                super::browser_worker::cancel(worker);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod browser_worker;
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    };
    /// Bound CPU checker workers across every view and live MCP endpoint in this process.
    #[cfg(test)]
    pub(super) static TEST_LOCK: Mutex<()> = Mutex::new(());
    pub(super) static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
    struct WorkerSlot;
    impl WorkerSlot {
        /// Refuse excess work immediately rather than allocating another worker thread.
        fn acquire() -> Option<Self> {
            let mut active = ACTIVE_WORKERS.load(Ordering::Acquire);
            loop {
                if active >= 2 {
                    return None;
                }
                match ACTIVE_WORKERS.compare_exchange_weak(active, active + 1, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => return Some(Self),
                    Err(current) => active = current,
                }
            }
        }
    }
    impl Drop for WorkerSlot {
        /// Release capacity on success, cancellation, spawn failure or worker unwind.
        fn drop(&mut self) {
            ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        }
    }
    pub(super) struct Running<J: Work> {
        id: u64,
        source: CheckSource,
        cancel: Arc<AtomicBool>,
        progress: Arc<Mutex<CheckProgress>>,
        result: Receiver<Result<Option<J::Output>, String>>,
        schedule_failure: Option<ScheduleFailure>,
    }
    impl<J: Work> Running<J> {
        /// Own a snapshot on one worker; its one-result channel never blocks publication.
        pub fn start(mut request: Request<J>) -> Self {
            let cancel = request.job.cancellation().unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
            let progress = Arc::new(Mutex::new(request.job.progress()));
            let (tx, result) = mpsc::sync_channel(1);
            let (stop, update) = (cancel.clone(), progress.clone());
            let source = request.job.source().clone();
            let id = request.id;
            let Some(slot) = WorkerSlot::acquire() else {
                let _ = tx.try_send(Err("Snapshot worker capacity reached; retry after a running job completes".into()));
                return Self { id, source, cancel, progress, result, schedule_failure: Some(ScheduleFailure::Capacity) };
            };
            let failed = tx.clone();
            let spawned = std::thread::Builder::new().name("pcbcraft-snapshot".into()).spawn(move || {
                let report = {
                    let _slot = slot;
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            break Ok(None);
                        }
                        let done = request.job.step(256);
                        if let Ok(mut p) = update.lock() {
                            *p = request.job.progress();
                        }
                        if done {
                            break request.job.take_output();
                        }
                    }
                };
                let _ = tx.try_send(report);
            });
            let schedule_failure = spawned.is_err().then_some(ScheduleFailure::Spawn);
            if let Err(error) = spawned {
                let _ = failed.try_send(Err(format!("Could not start snapshot job: {error}")));
            }
            Self { id, source, cancel, progress, result, schedule_failure }
        }
        /// Captured source, absent after an incremental request has been consumed.
        pub fn source(&self) -> Option<&CheckSource> {
            Some(&self.source)
        }
        /// Signal cooperative cancellation without waiting for the current batch.
        pub fn cancel(&mut self) {
            self.cancel.store(true, Ordering::Relaxed);
        }
        /// Copy the latest progress when available; never block the caller.
        pub fn progress(&self) -> Option<CheckProgress> {
            self.progress.try_lock().ok().map(|p| *p)
        }
        /// A stopped/panicked worker is a terminal failure, never an empty passing report.
        pub fn poll(&mut self) -> Option<Completion<J::Output>> {
            let report = match self.result.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Err("Snapshot worker stopped before completing".into()),
            };
            Some(Completion { id: self.id, source: self.source.clone(), report, schedule_failure: self.schedule_failure })
        }
    }

    impl<J: Work> Drop for Running<J> {
        /// Revoking a view never joins its worker during teardown.
        fn drop(&mut self) {
            self.cancel();
        }
    }
    #[cfg(test)]
    mod failure_tests {
        use super::*;
        /// A disconnected completion channel cannot be mistaken for a clean report.
        #[test]
        fn disconnected_worker_reports_failure() {
            let mut session = pcbcraft_engine::Session::default();
            session.apply("project.new", &serde_json::json!({"create_board":true})).unwrap();
            let job = session.design_check_job(&serde_json::json!({})).unwrap();
            let (tx, result) = mpsc::sync_channel(1);
            drop(tx);
            let mut running = Running::<CheckJob> {
                id: 1,
                source: job.source().clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                progress: Arc::new(Mutex::new(job.progress())),
                result,
                schedule_failure: None,
            };
            assert!(running.poll().unwrap().report.unwrap_err().contains("stopped"));
        }
        /// A real native copper worker returns a typed edit without changing the session;
        /// cancellation and an abandoned receiver release the shared worker permit.
        #[test]
        fn native_copper_worker_prepares_without_publication() {
            let _lock = TEST_LOCK.lock().unwrap();
            let mut session = pcbcraft_engine::Session::default();
            session.apply("project.new", &serde_json::json!({"create_board":true})).unwrap();
            session
                .apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &serde_json::json!({"copper_layers":2,"expected_revision":session.revision()}))
                .unwrap();
            let project = session.project().unwrap();
            let view = project.board_view(project.board_target().unwrap()).unwrap();
            let layer = view.layers().into_iter().find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper).unwrap().id;
            let params = serde_json::json!({"layer":layer,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":0},{"x_nm":1_000_000,"y_nm":0}],"expected_revision":session.revision()});
            let revision = session.revision();
            let job = session.prepare_copper_edit(pcbcraft_engine::ADD_TRACKS, &params).unwrap();
            let mut running = Running::start(Request { id: 1, job });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let completion = loop {
                if let Some(done) = running.poll() {
                    break done;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            };
            assert_eq!(session.revision(), revision);
            session.commit_copper_edit(completion.report.unwrap().unwrap().unwrap()).unwrap();
            assert_eq!(session.revision(), revision + 1);
            assert_eq!(ACTIVE_WORKERS.load(Ordering::Acquire), 0);
        }
        /// The native result channel preserves an engine refusal without publishing an edit.
        #[test]
        fn native_copper_worker_preserves_engine_failure() {
            let _lock = TEST_LOCK.lock().unwrap();
            let mut session = pcbcraft_engine::Session::default();
            session.apply("project.new", &serde_json::json!({"create_board":true})).unwrap();
            session
                .apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &serde_json::json!({"copper_layers":2,"expected_revision":session.revision()}))
                .unwrap();
            let before = session.inspect();
            let params = serde_json::json!({"tracks":[{"track":uuid::Uuid::from_u128(1),"start":{"x_nm":0,"y_nm":0},"end":{"x_nm":1_000_000,"y_nm":0}}],"expected_revision":session.revision()});
            let job = session.prepare_copper_edit(pcbcraft_engine::RESHAPE_TRACKS, &params).unwrap();
            let mut running = Running::start(Request { id: 1, job });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let completion = loop {
                if let Some(done) = running.poll() {
                    break done;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            };
            assert!(!completion.retryable());
            assert!(matches!(completion.report, Ok(Some(Err(pcbcraft_engine::CommandError::NotFound(message)))) if message == "unknown track"));
            assert_eq!(session.inspect(), before);
            assert_eq!(ACTIVE_WORKERS.load(Ordering::Acquire), 0);
        }
        /// Excess native work is refused without spawning a third worker or waiting.
        #[test]
        fn native_worker_capacity_is_bounded() {
            let _lock = TEST_LOCK.lock().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while ACTIVE_WORKERS.load(Ordering::Acquire) > 0 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            let first = WorkerSlot::acquire().unwrap();
            let second = WorkerSlot::acquire().unwrap();
            assert!(WorkerSlot::acquire().is_none());
            let mut session = pcbcraft_engine::Session::default();
            session.apply("project.new", &serde_json::json!({"create_board":true})).unwrap();
            let mut running = Running::start(Request { id: 1, job: session.design_check_job(&serde_json::json!({})).unwrap() });
            let completion = running.poll().unwrap();
            assert!(completion.retryable());
            assert_eq!(completion.schedule_failure, Some(ScheduleFailure::Capacity));
            let failure = crate::live_control::snapshot_worker_failure(completion.schedule_failure, completion.report.unwrap_err());
            assert_eq!(failure.code(), "busy");
            assert_eq!(failure.message(), "Snapshot worker capacity reached; retry after a running job completes");
            let spawn = crate::live_control::snapshot_worker_failure(Some(ScheduleFailure::Spawn), "Could not start snapshot job: test".into());
            assert_eq!(spawn.code(), "unavailable");
            assert_eq!(spawn.message(), "Could not start snapshot job: test");
            drop(first);
            drop(second);
            assert_eq!(ACTIVE_WORKERS.load(Ordering::Acquire), 0);
        }

        /// Canceling a fixed-pitch edit while its attached-track pass is still in progress
        /// releases the native snapshot worker slot and publishes no edit.
        #[test]
        fn native_pitch_cancel_during_carried_tracks_frees_worker_slot() {
            let _lock = TEST_LOCK.lock().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while ACTIVE_WORKERS.load(Ordering::Acquire) > 0 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            let mut session = pcbcraft_engine::Session::default();
            session.apply("project.new", &serde_json::json!({"create_board":true})).unwrap();
            session
                .apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &serde_json::json!({"copper_layers":2,"expected_revision":session.revision()}))
                .unwrap();
            for x in [0, 10_000_000, 20_000_000, 30_000_000] {
                let revision = session.revision();
                session.apply(pcbcraft_engine::PLACE_RESISTOR, &serde_json::json!({"x_nm":x,"y_nm":0,"expected_revision":revision})).unwrap();
                let symbol = session.project().unwrap().symbols().last().unwrap().id();
                let revision = session.revision();
                session
                    .apply(
                        pcbcraft_engine::PLACE_FOOTPRINT,
                        &serde_json::json!({"symbol":symbol,"definition":"header-1x02-254","mapping":{"1":"1","2":"2"},"x_nm":x,"y_nm":0,"expected_revision":revision}),
                    )
                    .unwrap();
            }
            let target = session.project().unwrap().board_target().unwrap();
            let layer = session
                .project()
                .unwrap()
                .board_view(target)
                .unwrap()
                .layers()
                .into_iter()
                .find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper)
                .unwrap()
                .id;
            for row in 0..6 {
                let y = 20_000_000 + row * 1_000_000;
                let points: Vec<serde_json::Value> = (0..256).map(|i| serde_json::json!({"x_nm":i * 1_000_000,"y_nm":y})).collect();
                session
                    .apply(
                        pcbcraft_engine::ADD_TRACKS,
                        &serde_json::json!({"board":target.board,"layer":layer,"width_nm":250_000,"points":points,"expected_revision":session.revision()}),
                    )
                    .unwrap();
            }
            let before = session.snapshot().unwrap();
            let revision = session.revision();
            let footprints: Vec<String> = before.board_view(target).unwrap().footprint_ids().collect();
            let params = serde_json::json!({
                "board": target.board,
                "footprints": footprints,
                "mode": "distribute_x",
                "pitch_nm": 5_080_000,
                "expected_revision": revision,
            });
            let mut job = session.prepare_copper_edit(pcbcraft_engine::ALIGN_FOOTPRINTS, &params).unwrap();
            // Advance one bounded step into the attached-track pass (still indexing moved
            // footprints), so the job is inside the resumable pass when handed to the native
            // worker and canceled.
            assert!(!job.step(1));
            assert_eq!(job.progress().stage, "Carrying attached tracks");
            assert!(job.progress().completed > 0);
            assert!(job.progress().completed < job.progress().total);

            let mut running = Running::start(Request { id: 1, job });
            running.cancel();
            let completion = loop {
                if let Some(done) = running.poll() {
                    break done;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            };
            assert!(matches!(completion.report, Ok(None)));
            assert_eq!(ACTIVE_WORKERS.load(Ordering::Acquire), 0);
            let reacquired = WorkerSlot::acquire();
            assert!(reacquired.is_some());
            drop(reacquired);
            assert_eq!(session.revision(), revision);
            assert!(Arc::ptr_eq(&session.snapshot().unwrap(), &before));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_engine::Session;
    use serde_json::json;
    /// A command-created empty board suffices to exercise scheduling without file surgery.
    fn request(session: &Session, id: u64) -> Request {
        Request { id, job: session.design_check_job(&json!({})).unwrap() }
    }
    /// Create a physical project through the same command used by GUI and MCP.
    fn session() -> Session {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board":true})).unwrap();
        s
    }

    /// A saturated queue retains only its latest replacement and never publishes partial work.
    #[test]
    fn replacement_is_bounded_and_canceled_completion_stays_distinct() {
        let s = session();
        let mut q = Queue::default();
        q.submit(request(&s, 1));
        for id in 2..100 {
            q.submit(request(&s, id));
            assert!(q.active.is_some());
            assert!(q.pending.is_some());
        }
        assert_eq!(q.replaced, 97);
        let old = q.poll().unwrap();
        assert_eq!(old.id, 1);
        assert!(old.report.unwrap().is_none());
        let mut done = None;
        for _ in 0..100 {
            if let Some(result) = q.poll() {
                done = Some(result);
                break;
            }
        }
        let done = done.unwrap();
        assert_eq!(done.id, 99);
        assert!(done.report.unwrap().is_some());
        assert!(!q.busy());
        q.submit(request(&s, 100));
        q.cancel();
        assert!(q.poll().unwrap().report.unwrap().is_none());
        assert!(!q.busy());
    }

    /// The real native worker reports the same snapshot, supports cancellation and never needs
    /// a UI join. Dropping its receiver makes result publication harmless.
    #[test]
    fn native_worker_completion_cancellation_and_receiver_drop() {
        let _lock = native::TEST_LOCK.lock().unwrap();
        use std::time::{Duration, Instant};
        let s = session();
        let mut running = native::Running::start(request(&s, 1));
        assert!(running.source().unwrap().is_current(&s));
        let _ = running.progress();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(done) = running.poll() {
                assert!(done.report.unwrap().is_some());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut running = native::Running::start(request(&s, 2));
        running.cancel();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(done) = running.poll() {
                assert!(done.report.is_ok());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(native::Running::start(request(&s, 3)));
        let deadline = Instant::now() + Duration::from_secs(5);
        while native::ACTIVE_WORKERS.load(std::sync::atomic::Ordering::Acquire) > 0 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}
