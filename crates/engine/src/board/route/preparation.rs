//! Captured walkaround work, independent of view widgets and publication.
use super::*;
use crate::board::{
    clearance::preparation::Preparation,
    view::BoardTarget,
    work::{Progress, Source},
};
use std::sync::{Arc, Mutex, TryLockError};

/// One gesture's immutable query projection, retained across coalesced pointer requests.
#[derive(Clone)]
pub struct PlanningContext {
    source: Source,
    preparation: Arc<Mutex<Preparation>>,
}
impl PlanningContext {
    /// Captured session, opening, board and revision; hosts revoke obsolete view generations.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Capture read-only work only when the obstacle query is already prepared. A busy/cold
    /// context returns None without building or waiting. The owner must check source authority
    /// before displaying/adopting results; this capture grants no document edit authority.
    pub fn try_prepare_cached(&self, request: Request, budget: usize) -> Result<Option<PlanningJob>, CommandError> {
        let query = match self.preparation.try_lock() {
            Ok(preparation) => preparation.cached()?,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => return Err(CommandError::Unavailable("Route obstacle preparation failed".into())),
        };
        let Some(query) = query else {
            return Ok(None);
        };
        super::validate(&query, &request, budget)?;
        Ok(Some(PlanningJob {
            context: self.clone(),
            search: Some(search::Search::new(query, request.clone(), budget)),
            request,
            budget,
            result: None,
            done: false,
        }))
    }
}

/// A resumable search. Dropping it cancels private work without publishing any copper.
pub struct PlanningJob {
    context: PlanningContext,
    request: Request,
    budget: usize,
    search: Option<search::Search>,
    result: Option<Result<Plan, CommandError>>,
    done: bool,
}
impl Session {
    /// Capture a gesture without projecting geometry or serializing its immutable project.
    pub fn route_planning(&self, target: BoardTarget) -> Result<PlanningContext, CommandError> {
        let source = self.copper_preparation(target)?.source().clone();
        Ok(PlanningContext { preparation: Arc::new(Mutex::new(Preparation::new(source.clone()))), source })
    }
    /// Refuse a stale gesture before scheduling; full geometry/policy checks run in the job.
    pub fn prepare_route_plan(&self, context: &PlanningContext, request: Request, budget: usize) -> Result<PlanningJob, CommandError> {
        if !context.source.is_current(self) {
            return Err(CommandError::InvalidParams("route planning source is stale".into()));
        }
        if !(1..=MAX_BUDGET).contains(&budget) || request.previous.len() > super::super::track::MAX_PATH_POINTS {
            return Err(CommandError::InvalidParams("invalid route planning bound".into()));
        }
        Ok(PlanningJob { context: context.clone(), request, budget, search: None, result: None, done: false })
    }
}
impl PlanningJob {
    /// Captured authority accompanies completion and must still be current before adoption.
    pub fn source(&self) -> &Source {
        &self.context.source
    }
    /// Exact search probes completed; geometry preparation is reported separately.
    pub fn progress(&self) -> Progress {
        Progress {
            stage: if self.search.is_some() { "Planning route" } else { "Preparing route obstacles" },
            completed: self.search.as_ref().map_or(0, search::Search::probes),
            total: self.budget,
        }
    }
    /// Advance bounded contact/search units. A contended context yields without waiting;
    /// poisoned preparation fails this request rather than blocking other editor work.
    pub fn step(&mut self, budget: usize) -> bool {
        if self.done {
            return true;
        }
        if budget == 0 {
            return false;
        }
        if self.search.is_none() {
            let result = match self.context.preparation.try_lock() {
                Ok(mut preparation) => preparation.step(budget),
                Err(TryLockError::WouldBlock) => return false,
                Err(TryLockError::Poisoned(_)) => Err(CommandError::Unavailable("Route obstacle preparation failed".into())),
            };
            match result {
                Ok(Some(query)) => self.search = Some(search::Search::new(query, self.request.clone(), self.budget)),
                Ok(None) => return false,
                Err(error) => {
                    self.result = Some(Err(error));
                    self.done = true;
                }
            }
            return self.done;
        }
        if let Some(result) = self.search.as_mut().and_then(|search| search.step(budget.min(8))) {
            self.result = Some(result);
            self.done = true;
        }
        self.done
    }
    /// Consume only a terminal result; no partial path is exposed as a valid preview.
    pub fn take_result(&mut self) -> Result<Option<Plan>, CommandError> {
        self.result.take().transpose()
    }
}
