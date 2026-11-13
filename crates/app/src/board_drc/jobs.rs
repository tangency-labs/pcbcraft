//! Bounded per-view scheduling: one running snapshot and one replaceable pending snapshot.
use pcbcraft_engine::board::drc::{CheckJob, CheckProgress, CheckSource, Report};

/// One immutable checker request tagged with its owner's publication generation.
pub(crate) struct Request {
    pub id: u64,
    pub job: CheckJob,
}
/// A terminal completion carries its source; canceled work contains no report.
pub(crate) struct Completion {
    pub id: u64,
    pub source: CheckSource,
    pub report: Result<Option<Report>, String>,
}
/// Bounded scheduling retains one active and one replaceable pending request per owner.
#[derive(Default)]
pub(crate) struct Queue {
    active: Option<Running>,
    pending: Option<Request>,
    pub replaced: u64,
    pub canceled: u64,
    pub completed: u64,
}
impl std::fmt::Debug for Queue {
    /// Debug output exposes occupancy without retaining or formatting project contents.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckerQueue").field("running", &self.active.is_some()).field("pending", &self.pending.is_some()).finish()
    }
}
impl Queue {
    /// Coalesce replacements without starting another worker until cancellation is acknowledged.
    pub fn submit(&mut self, request: Request) {
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
    pub fn poll(&mut self) -> Option<Completion> {
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
    pub(super) struct Running {
        request: Option<Request>,
        canceled: bool,
    }
    impl Running {
        /// Retain the captured engine job directly; browser execution serializes no project.
        pub fn start(request: Request) -> Self {
            Self { request: Some(request), canceled: false }
        }
        /// Captured source, absent after an incremental request has been consumed.
        pub fn source(&self) -> Option<&CheckSource> {
            self.request.as_ref().map(|r| r.job.source())
        }
        /// Signal cooperative cancellation without waiting for the current batch.
        pub fn cancel(&mut self) {
            self.canceled = true;
        }
        /// Copy the latest progress when available; never block the caller.
        pub fn progress(&self) -> Option<CheckProgress> {
            self.request.as_ref().map(|r| r.job.progress())
        }
        /// Advance one engine batch, publishing only a complete report or cancellation.
        fn poll_batch(&mut self) -> Option<Completion> {
            let request = self.request.as_mut()?;
            if !self.canceled && !request.job.step(256) {
                return None;
            }
            let mut request = self.request.take()?;
            Some(Completion {
                id: request.id,
                source: request.job.source().clone(),
                report: Ok(if self.canceled { None } else { request.job.take_report() }),
            })
        }

        /// Yield after two browser milliseconds; tests use one deterministic batch per poll.
        pub fn poll(&mut self) -> Option<Completion> {
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
}
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
    static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
    struct WorkerSlot;
    impl WorkerSlot {
        /// Refuse excess work immediately rather than allocating another worker thread.
        fn acquire() -> Option<Self> {
            ACTIVE_WORKERS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < 2).then_some(n + 1)).ok().map(|_| Self)
        }
    }
    impl Drop for WorkerSlot {
        /// Release capacity on success, cancellation, spawn failure or worker unwind.
        fn drop(&mut self) {
            ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        }
    }
    pub(super) struct Running {
        id: u64,
        source: CheckSource,
        cancel: Arc<AtomicBool>,
        progress: Arc<Mutex<CheckProgress>>,
        result: Receiver<Result<Option<Report>, String>>,
    }
    impl Running {
        /// Own a snapshot on one worker; its one-result channel never blocks publication.
        pub fn start(mut request: Request) -> Self {
            let cancel = Arc::new(AtomicBool::new(false));
            let progress = Arc::new(Mutex::new(request.job.progress()));
            let (tx, result) = mpsc::sync_channel(1);
            let (stop, update) = (cancel.clone(), progress.clone());
            let source = request.job.source().clone();
            let id = request.id;
            let Some(slot) = WorkerSlot::acquire() else {
                let _ = tx.try_send(Err("Checker worker capacity reached; retry after a running check completes".into()));
                return Self { id, source, cancel, progress, result };
            };
            let failed = tx.clone();
            let spawned = std::thread::Builder::new().name("pcbcraft-checker".into()).spawn(move || {
                let report = {
                    let _slot = slot;
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            break None;
                        }
                        let done = request.job.step(256);
                        if let Ok(mut p) = update.lock() {
                            *p = request.job.progress();
                        }
                        if done {
                            break request.job.take_report();
                        }
                    }
                };
                let _ = tx.try_send(Ok(report));
            });
            if let Err(error) = spawned {
                let _ = failed.try_send(Err(format!("Could not start checker: {error}")));
            }
            Self { id, source, cancel, progress, result }
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
        pub fn poll(&mut self) -> Option<Completion> {
            let report = match self.result.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Err("Checker worker stopped before completing".into()),
            };
            Some(Completion { id: self.id, source: self.source.clone(), report })
        }
    }

    impl Drop for Running {
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
            let mut running = Running {
                id: 1,
                source: job.source().clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                progress: Arc::new(Mutex::new(job.progress())),
                result,
            };
            assert!(running.poll().unwrap().report.unwrap_err().contains("stopped"));
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
            assert!(running.poll().unwrap().report.unwrap_err().contains("capacity"));
            drop(first);
            drop(second);
            assert_eq!(ACTIVE_WORKERS.load(Ordering::Acquire), 0);
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
    }
}
