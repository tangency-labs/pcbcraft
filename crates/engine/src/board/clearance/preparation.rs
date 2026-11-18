//! Reusable, bounded contact projection for captured route queries.
use super::*;
use crate::board::{
    track::{Copper, IslandBuilder, lands_from_footprints},
    work::Source,
};

/// Shared projection survives superseded pointer work; a canceled search publishes nothing.
pub(crate) struct Preparation {
    source: Source,
    phase: u8,
    next: usize,
    lands: Vec<Land>,
    builder: Option<IslandBuilder>,
    copper: Copper,
    members: Vec<Vec<usize>>,
    ready: Option<Arc<ClearanceQuery>>,
    failed: Option<String>,
}
impl Preparation {
    /// Read only an already prepared query; never project geometry or wait for another builder.
    pub(crate) fn cached(&self) -> Result<Option<Arc<ClearanceQuery>>, CommandError> {
        if let Some(error) = &self.failed {
            return Err(CommandError::Unavailable(error.clone()));
        }
        Ok(self.ready.clone())
    }
    /// Capture authority without projecting pads or constructing connectivity on the caller.
    pub(crate) fn new(source: Source) -> Self {
        Self {
            source,
            phase: 0,
            next: 0,
            lands: Vec::new(),
            builder: None,
            copper: Copper::default(),
            members: Vec::new(),
            ready: None,
            failed: None,
        }
    }
    /// Project at most 32 footprints or `budget` contact pairs; yield between phase changes.
    /// Final contour/class/index construction has the same supported model bounds as inspection.
    pub(crate) fn step(&mut self, budget: usize) -> Result<Option<Arc<ClearanceQuery>>, CommandError> {
        if let Some(error) = &self.failed {
            return Err(CommandError::Unavailable(error.clone()));
        }
        if let Some(query) = &self.ready {
            return Ok(Some(query.clone()));
        }
        if budget == 0 {
            return Ok(None);
        }
        let view = self.source.snapshot.board_view(self.source.target).ok_or_else(|| CommandError::Unavailable("Board unavailable".into()))?;
        match self.phase {
            0 => {
                self.lands.extend(lands_from_footprints(view.footprints_range(self.next, budget.min(32))));
                self.next = self.next.saturating_add(budget.min(32));
                if self.next < self.source.snapshot.board.footprints.len() {
                    return Ok(None);
                }
                self.builder = Some(IslandBuilder::new(&self.lands, view.tracks(), view.vias()));
            }
            1 => {
                if !self.builder.as_mut().is_some_and(|builder| builder.step(budget)) {
                    return Ok(None);
                }
                if let Some(builder) = self.builder.take() {
                    (self.copper, self.members) = builder.finish(&self.lands, view.tracks(), view.vias());
                }
                self.builder = Some(IslandBuilder::pad_contacts(&self.lands, view.tracks(), view.vias(), &self.members));
            }
            _ => {
                if !self.builder.as_mut().is_some_and(|builder| builder.step_pads(self.lands.len(), budget)) {
                    return Ok(None);
                }
                if let Some(builder) = self.builder.take() {
                    let (_, merged) = builder.finish(&self.lands, view.tracks(), view.vias());
                    let geometry = Arc::new(BoardGeometry::from_contacts(
                        view,
                        std::mem::take(&mut self.lands),
                        std::mem::take(&mut self.copper),
                        std::mem::take(&mut self.members),
                        merged,
                    ));
                    match ClearanceQuery::from_geometry(&self.source, geometry) {
                        Ok(query) => self.ready = Some(Arc::new(query)),
                        Err(error) => {
                            self.failed = Some(error.to_string());
                            return Err(error);
                        }
                    }
                }
                return Ok(self.ready.clone());
            }
        }
        self.phase += 1;
        Ok(None)
    }
}
