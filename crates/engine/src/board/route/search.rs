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

pub(super) use crate::board::work::yield_unit;

/// Retain the walk's own stack and local geometry across budget boundaries without replay.
pub(super) async fn walk(
    query: crate::board::clearance::RouteObstacles<'_>,
    request: &Request,
    budget: usize,
    progress: Arc<AtomicUsize>,
) -> Result<Plan, CommandError> {
    let mut walker = walkaround::Walker::new(query, request, budget, progress)?;
    match walker.run_target(request).await {
        Ok(mut plan) => {
            if request.mode == Mode::Walkaround
                && !plan.start_blocked
                && plan.points.len() >= 2
                && let Outcome::Blocked { last_legal } = plan.outcome
            {
                // The whole prefix was exact-confirmed, even if no nearby-goal refinement
                // succeeded. Its displayed endpoint is still a valid vertex to fix.
                plan.adjusted_target = Some(last_legal);
            }
            Ok(plan)
        }
        Err(walkaround::Stop::Exhausted) => Ok(Plan {
            adjusted_target: None,
            start_blocked: false,
            via_moves: Vec::new(),
            replacements: Vec::new(),
            shove: None,
            points: vec![request.head],
            outcome: Outcome::Incomplete,
            blocker: None,
            probes: walker.probes(),
        }),
        Err(walkaround::Stop::Failed(error)) => Err(error),
    }
}

/// Select a read-only solver for inputs already validated by the caller.
async fn run(query: &ClearanceQuery, request: &Request, budget: usize, progress: Arc<AtomicUsize>) -> Result<Plan, CommandError> {
    match request.mode {
        Mode::Walkaround => walk(query.into(), request, budget, progress).await,
        Mode::Shove | Mode::ShoveVias => shove::run(query, request, budget, progress).await,
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
            return Ok(Plan {
                adjusted_target: None,
                start_blocked: false,
                via_moves: Vec::new(),
                replacements: Vec::new(),
                shove: None,
                points: vec![request.head],
                outcome: Outcome::Canceled,
                blocker: None,
                probes: progress.load(Ordering::Relaxed),
            });
        }
        if let Poll::Ready(plan) = future.as_mut().poll(&mut cx) {
            return plan;
        }
    }
}

/// Owned continuation suitable for either the native snapshot worker or browser frame queue.
pub(in crate::board) struct Search {
    future: Pin<Box<dyn Future<Output = Result<Plan, CommandError>> + Send>>,
    progress: Arc<AtomicUsize>,
}
impl Search {
    /// Own captured inputs so suspension cannot borrow a live editor/session.
    pub(super) fn new(query: Arc<ClearanceQuery>, request: Request, budget: usize) -> Self {
        let progress = Arc::new(AtomicUsize::new(0));
        let counter = progress.clone();
        let future = Box::pin(async move {
            super::validate(&query, &request, budget)?;
            run(&query, &request, budget, counter).await
        });
        Self { future, progress }
    }
    /// Own the drag input across cooperative host steps, just like a route gesture.
    pub(in crate::board) fn drag(query: Arc<ClearanceQuery>, ends: Vec<crate::board::track::Ends>, mode: Mode) -> Self {
        let progress = Arc::new(AtomicUsize::new(0));
        let counter = progress.clone();
        let future = Box::pin(async move { shove::drag(&query, ends, mode, MAX_BUDGET, counter).await });
        Self { future, progress }
    }
    /// Exact probes completed so far, independent of host scheduling frequency.
    pub(super) fn probes(&self) -> usize {
        self.progress.load(Ordering::Relaxed)
    }
    /// Advance at most the supplied suspension boundaries; partial paths stay private.
    pub(in crate::board) fn step(&mut self, budget: usize) -> Option<Result<Plan, CommandError>> {
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..budget {
            if let Poll::Ready(plan) = self.future.as_mut().poll(&mut cx) {
                return Some(plan);
            }
        }
        None
    }
}
