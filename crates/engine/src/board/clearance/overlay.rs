//! Small gesture-local changes over an immutable indexed board; dropping this value cancels it.
use super::*;
use std::collections::BTreeMap;

/// Stable within a captured query/overlay. Source positions expire with its authority;
/// added objects use caller-owned UUIDs and never alias source positions or saved net IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObstacleId {
    Source(usize),
    Added(Uuid),
}

/// Exact decision against an addressed overlay obstacle, without a legacy island exemption.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayDecision {
    pub obstacle: ObstacleId,
    pub gap_nm: f64,
    pub touches: bool,
    pub clearance_violation: bool,
    pub conflicting_nets: bool,
}

/// Overlay decisions retain source authority; callers still check current gesture ownership.
pub struct OverlayResults {
    pub authority: Authority,
    pub decisions: Vec<OverlayDecision>,
    overlay: Arc<()>,
    generation: u64,
}

impl OverlayResults {
    /// Reject changed/canceled overlays as well as changed source sessions before reuse.
    pub fn is_current(&self, overlay: &Overlay<'_>, session: &Session) -> bool {
        self.authority.is_current(session) && Arc::ptr_eq(&self.overlay, &overlay.identity) && self.generation == overlay.generation
    }
}

/// Disposable query data, never authored geometry or an authorization to publish an edit.
/// Up to 8,192 changed/added objects are retained; the source index is never modified/rebuilt.
pub struct Overlay<'a> {
    query: &'a ClearanceQuery,
    changed: BTreeMap<usize, Option<Object>>,
    added: BTreeMap<Uuid, Object>,
    identity: Arc<()>,
    generation: u64,
}

impl ClearanceQuery {
    /// Start an empty disposable overlay sharing this captured board's index and authority.
    pub fn overlay(&self) -> Overlay<'_> {
        Overlay { query: self, changed: BTreeMap::new(), added: BTreeMap::new(), identity: Arc::new(()), generation: 0 }
    }
}

impl Overlay<'_> {
    /// Advance only after input validation, before mutation; exhaustion cannot wrap authority.
    fn advance(&mut self) -> Result<(), CommandError> {
        self.generation = self.generation.checked_add(1).ok_or_else(|| CommandError::Unavailable("overlay generation exhausted".into()))?;
        Ok(())
    }
    /// Replacing a previous change consumes no extra slot; reject excess work before mutation.
    fn room(&self, existing: bool) -> Result<(), CommandError> {
        if !existing && self.changed.len() + self.added.len() >= 8192 {
            return Err(CommandError::Unavailable("overlay supports at most 8192 changed objects".into()));
        }
        Ok(())
    }

    /// Resolve one original identity without letting a deleted source become a different object.
    fn source(&self, source: usize) -> Result<&Object, CommandError> {
        self.query.objects().get(source).ok_or_else(|| CommandError::InvalidParams("unknown source copper object".into()))
    }

    /// Hide a source obstacle. Repeated deletion is idempotent and leaves the base untouched.
    pub fn remove(&mut self, source: usize) -> Result<(), CommandError> {
        self.source(source)?;
        self.room(self.changed.contains_key(&source))?;
        self.advance()?;
        self.changed.insert(source, None);
        Ok(())
    }

    /// Replace source geometry/layers under the same identity and intended membership.
    /// Validation precedes mutation; prior replacements and removals can be replaced again.
    pub fn replace(&mut self, source: usize, contour: Contour, layers: Vec<Uuid>) -> Result<(), CommandError> {
        self.query.validate_layers(&layers)?;
        let mut item = self.source(source)?.clone();
        self.room(self.changed.contains_key(&source))?;
        self.advance()?;
        item.contour = contour;
        item.layers = layers;
        item.at = Point {
            x_nm: item.contour.core.iter().map(|p| p.x_nm).sum::<i64>() / 4,
            y_nm: item.contour.core.iter().map(|p| p.y_nm).sum::<i64>() / 4,
        };
        self.changed.insert(source, Some(item));
        Ok(())
    }

    /// Add one independent UUID-addressed obstacle, optionally inheriting source intent.
    /// Duplicate IDs, invalid layers/sources and excess work leave the overlay unchanged.
    pub fn add(&mut self, id: Uuid, contour: Contour, layers: Vec<Uuid>, intended_from: Option<usize>) -> Result<(), CommandError> {
        self.query.validate_layers(&layers)?;
        if self.added.contains_key(&id) {
            return Err(CommandError::InvalidParams("duplicate overlay object identity".into()));
        }
        let nets = match intended_from {
            Some(source) => self.source(source)?.intended_nets.clone(),
            None => Vec::new(),
        };
        self.room(false)?;
        self.advance()?;
        let at = contour.core[0];
        self.added.insert(id, Object { name: id.to_string(), contour, layers, intended_nets: nets, at, island: usize::MAX, net: None });
        Ok(())
    }

    /// Delete a previously added obstacle, releasing its overlay slot without editing the model.
    pub fn remove_added(&mut self, id: Uuid) -> Result<(), CommandError> {
        if !self.added.contains_key(&id) {
            return Err(CommandError::InvalidParams("unknown overlay object identity".into()));
        }
        self.advance()?;
        self.added.remove(&id);
        Ok(())
    }

    /// Resolve current overlay geometry; removed originals are absent rather than superseded hits.
    pub fn object(&self, id: ObstacleId) -> Option<&Object> {
        match id {
            ObstacleId::Source(i) => match self.changed.get(&i) {
                Some(item) => item.as_ref(),
                None => self.query.objects().get(i),
            },
            ObstacleId::Added(id) => self.added.get(&id),
        }
    }

    /// Merge indexed base hits with changed/added envelopes in deterministic identity order.
    /// Small overlays are scanned; no base copy, tree update or serialization is performed.
    pub fn candidates(&self, contour: &Contour, layers: &[Uuid]) -> Result<Vec<ObstacleId>, CommandError> {
        let mut ids: Vec<_> =
            self.query.candidates(contour, layers)?.into_iter().filter(|i| !self.changed.contains_key(i)).map(ObstacleId::Source).collect();
        let near = |item: &Object| {
            share_layers(layers, &item.layers) && near_bounds(contour.bounds(), item.contour.bounds(), self.query.authority.clearance_nm as f64)
        };
        ids.extend(self.changed.iter().filter_map(|(i, item)| item.as_ref().filter(|item| near(item)).map(|_| ObstacleId::Source(*i))));
        ids.extend(self.added.iter().filter_map(|(id, item)| near(item).then_some(ObstacleId::Added(*id))));
        ids.sort_unstable();
        Ok(ids)
    }

    /// Apply the shared exact predicates after candidate lookup, combining all contacted intent.
    /// An inherited object is replaced by the probe and excluded, including its old location.
    pub fn probe(&self, contour: &Contour, layers: &[Uuid], intended_from: Option<ObstacleId>) -> Result<OverlayResults, CommandError> {
        let nets = match intended_from {
            Some(id) => self.object(id).ok_or_else(|| CommandError::InvalidParams("unknown overlay intent object".into()))?.intended_nets.clone(),
            None => Vec::new(),
        };
        let gaps: Vec<_> = self
            .candidates(contour, layers)?
            .into_iter()
            .filter(|id| Some(*id) != intended_from)
            .filter_map(|id| self.object(id).map(|item| (id, item, contour.gap(&item.contour))))
            .collect();
        let connected: Vec<_> = nets
            .into_iter()
            .chain(gaps.iter().filter(|(_, _, gap)| *gap <= 0.).flat_map(|(_, item, _)| item.intended_nets.iter().copied()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let decisions = gaps
            .into_iter()
            .map(|(id, item, gap)| {
                let d = decision(None, 0, gap, &connected, &item.intended_nets, false, self.query.authority.clearance_nm);
                OverlayDecision {
                    obstacle: id,
                    gap_nm: d.gap_nm,
                    touches: d.touches,
                    clearance_violation: d.clearance_violation,
                    conflicting_nets: d.conflicting_nets,
                }
            })
            .collect();
        Ok(OverlayResults { authority: self.query.authority.clone(), decisions, overlay: self.identity.clone(), generation: self.generation })
    }
}
