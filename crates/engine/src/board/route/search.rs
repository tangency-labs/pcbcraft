//! Manually polled search continuation, shared by synchronous callers and bounded hosts.
use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

/// Suspend before one exact probe or contact scan; the host explicitly drives the next unit.
/// No executor, thread, timer, wake callback or shared mutable document is needed.
pub(super) async fn yield_unit() {
    let mut yielded = false;
    std::future::poll_fn(move |_| if std::mem::replace(&mut yielded, true) { Poll::Ready(()) } else { Poll::Pending }).await
}

/// Retain the walk's own stack and local geometry across budget boundaries without replay.
async fn run(query: &ClearanceQuery, request: &Request, budget: usize, progress: Arc<AtomicUsize>) -> Result<Plan, CommandError> {
    super::validate(query, request, budget)?;
    let mut walker = walkaround::Walker::new(query, request, budget, progress)?;
    match walker.run(request).await {
        Ok(plan) => Ok(plan),
        Err(walkaround::Stop::Exhausted) => {
            Ok(Plan { points: vec![request.head], outcome: Outcome::Incomplete, blocker: None, probes: walker.probes() })
        }
        Err(walkaround::Stop::Failed(error)) => Err(error),
    }
}

/// Drain the same continuation for existing synchronous clients, checking cancellation at
/// every work boundary. Cancellation never returns a partially validated path.
pub(super) fn complete(query: &ClearanceQuery, request: &Request, budget: usize, cancel: &dyn Fn() -> bool) -> Result<Plan, CommandError> {
    let progress = Arc::new(AtomicUsize::new(0));
    let mut future = Box::pin(run(query, request, budget, progress.clone()));
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if cancel() {
            return Ok(Plan { points: vec![request.head], outcome: Outcome::Canceled, blocker: None, probes: progress.load(Ordering::Relaxed) });
        }
        if let Poll::Ready(plan) = future.as_mut().poll(&mut cx) {
            return plan;
        }
    }
}

/// Owned continuation suitable for either the native snapshot worker or browser frame queue.
pub(super) struct Search {
    future: Pin<Box<dyn Future<Output = Result<Plan, CommandError>> + Send>>,
    progress: Arc<AtomicUsize>,
}
impl Search {
    /// Own captured inputs so suspension cannot borrow a live editor/session.
    pub(super) fn new(query: Arc<ClearanceQuery>, request: Request, budget: usize) -> Self {
        let progress = Arc::new(AtomicUsize::new(0));
        let counter = progress.clone();
        Self { future: Box::pin(async move { run(&query, &request, budget, counter).await }), progress }
    }
    /// Exact probes completed so far, independent of host scheduling frequency.
    pub(super) fn probes(&self) -> usize {
        self.progress.load(Ordering::Relaxed)
    }
    /// Advance at most the supplied suspension boundaries; partial paths stay private.
    pub(super) fn step(&mut self, budget: usize) -> Option<Result<Plan, CommandError>> {
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..budget {
            if let Poll::Ready(plan) = self.future.as_mut().poll(&mut cx) {
                return Some(plan);
            }
        }
        None
    }
}
