//! Transactional continuation evidence comes from frozen before/after graphs, never saved members.

use super::{Anchor, Lineage, MAX_RECORDS, Record, Registry, Retirement, Scope, State, graph::Graph};
use crate::{CommandError, Project, board::update::SourceCapture};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

/// Allocate independently while refusing every live, ambiguous and retired registry UUID.
pub(super) fn allocate(ids: &mut BTreeSet<Uuid>) -> Result<Uuid, CommandError> {
    if ids.len() >= MAX_RECORDS {
        return Err(CommandError::Unavailable("Net identity registry is full.".into()));
    }
    for _ in 0..4 {
        let id = crate::fresh_id()?;
        if ids.insert(id) {
            return Ok(id);
        }
    }
    Err(CommandError::Unavailable("Could not allocate a distinct net identity.".into()))
}

/// Capture once only when an authored edit actually needs historical lineage/disposition.
fn source_capture(capture: &mut Option<SourceCapture>, before: &Project) -> Result<SourceCapture, CommandError> {
    if let Some(capture) = capture {
        return Ok(capture.clone());
    }
    let saved = crate::board::update::capture_source(before)?;
    *capture = Some(saved.clone());
    Ok(saved)
}

/// Current instance storage follows a first-board scope transfer; lineage keeps its captured owner.
pub(super) fn requalify(records: &mut Registry, scope: Scope) {
    for record in &mut records.records {
        if !matches!(record.state, State::Active {}) {
            continue;
        }
        record.primary.scope = scope;
        for anchor in &mut record.fallbacks {
            anchor.scope = scope;
        }
    }
}

/// Choose only unique persisted anchor evidence, never largest membership or traversal order.
fn continuation(record: &Record, targets: &[usize], after: &Graph) -> Option<usize> {
    if targets.len() == 1 {
        return targets.first().copied();
    }
    if let Some(index) = after.network(&record.primary).filter(|index| targets.contains(index)) {
        return Some(index);
    }
    let fallbacks: BTreeSet<_> = record.fallbacks.iter().filter_map(|a| after.network(a)).filter(|index| targets.contains(index)).collect();
    if fallbacks.len() == 1 { fallbacks.first().copied() } else { None }
}

/// Keep deterministic unique references without storing any endpoint membership on a record.
fn unique(ids: impl IntoIterator<Item = Uuid>) -> Vec<Uuid> {
    ids.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

/// Preserve source-qualified identity evidence even when first-board creation changes live scope.
fn source_anchor(anchor: &Anchor, scope: Scope) -> Anchor {
    Anchor { scope, ..anchor.clone() }
}

impl Project {
    /// Reconcile one canonical candidate before history publication. Non-anchor changes keep exact
    /// registry bytes; allocation/validation failure leaves the owning session untouched.
    pub(crate) fn reconcile_nets(&mut self, before: Option<&Project>) -> Result<(), CommandError> {
        let Some(before) = before.filter(|before| before.same_document(self)) else {
            return Ok(());
        };
        let old_scope = before.net_scope();
        let scope = self.net_scope();
        if old_scope == scope
            && Arc::ptr_eq(&before.sheets, &self.sheets)
            && Arc::ptr_eq(&before.units, &self.units)
            && Arc::ptr_eq(&before.components, &self.components)
            && Arc::ptr_eq(&before.net_registry, &self.net_registry)
        {
            return Ok(());
        }
        let mut old = Graph::build(before).map_err(CommandError::InvalidParams)?;
        let after = Graph::build(self).map_err(CommandError::InvalidParams)?;
        let mut registry = (*before.net_registry).clone();
        if old_scope != scope {
            old.requalify(scope);
            requalify(&mut registry, scope);
        }
        if old.networks == after.networks {
            let mut explicit = (*self.net_registry).clone();
            if old_scope != scope {
                requalify(&mut explicit, scope);
            }
            if explicit != *self.net_registry {
                self.net_registry = Arc::new(explicit);
            }
            if !self.net_registry.records.is_empty() {
                self.net_registry.validate(self).map_err(CommandError::InvalidParams)?;
            }
            return Ok(());
        }
        before.net_registry.validate(before).map_err(CommandError::InvalidParams)?;
        let mut by_source = BTreeMap::<usize, Record>::new();
        for record in &registry.records {
            if matches!(record.state, State::Active {}) {
                let index = old.network(&record.primary).ok_or_else(|| CommandError::InvalidParams("Missing source net primary.".into()))?;
                by_source.insert(index, record.clone());
            }
        }
        let targets: Vec<Vec<usize>> = old
            .networks
            .iter()
            .map(|network| {
                if !network.meaningful() {
                    return Vec::new();
                }
                network
                    .anchors
                    .iter()
                    .filter_map(|a| after.network(a))
                    .filter(|i| after.networks[*i].meaningful())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .collect();
        let mut sources = vec![Vec::new(); after.networks.len()];
        for (index, branches) in targets.iter().enumerate() {
            for branch in branches {
                sources[*branch].push(index);
            }
        }
        let chosen: BTreeMap<_, _> = by_source.iter().map(|(index, r)| (r.id, continuation(r, &targets[*index], &after))).collect();
        let mut result: BTreeMap<_, _> = registry.records.iter().map(|r| (r.id, r.clone())).collect();
        let mut ids = result.keys().copied().collect();
        let mut assigned = vec![None; after.networks.len()];
        let mut capture = None;
        for (index, network) in after.networks.iter().enumerate().filter(|(_, n)| n.meaningful()) {
            let parents = &sources[index];
            let persistent = parents.iter().any(|source| by_source.contains_key(source));
            if !persistent && network.names.is_empty() {
                continue;
            }
            let retained = if parents.len() == 1 {
                parents.first().and_then(|source| by_source.get(source)).filter(|r| chosen.get(&r.id) == Some(&Some(index)))
            } else {
                None
            };
            let mut record = if let Some(retained) = retained {
                network.retained(retained).map_err(CommandError::InvalidParams)?
            } else {
                let mut record = network.record(allocate(&mut ids)?, None).map_err(CommandError::InvalidParams)?;
                if persistent || parents.len() > 1 {
                    let snapshot = source_capture(&mut capture, before)?;
                    for source in parents {
                        let primary = by_source
                            .get(source)
                            .map(|r| &r.primary)
                            .or_else(|| old.networks[*source].anchors.first())
                            .ok_or_else(|| CommandError::InvalidParams("Source network has no identity anchor.".into()))?;
                        record.lineage.push(Lineage {
                            snapshot: snapshot.clone(),
                            net: by_source.get(source).map(|r| r.id),
                            anchor: source_anchor(primary, old_scope),
                        });
                    }
                }
                record
            };
            let mut inherited = BTreeSet::new();
            for parent in parents {
                if let Some(source) = by_source.get(parent) {
                    for binding in &source.class_bindings {
                        let class =
                            self.net_classes.get(binding.class).ok_or_else(|| CommandError::InvalidParams("Unknown inherited class.".into()))?;
                        if targets[*parent].len() > 1 && class.rules.keys().any(|property| !property.shareable_on_split()) {
                            return Err(CommandError::Unavailable("A class property does not support automatic split inheritance.".into()));
                        }
                        inherited.insert(binding.clone());
                        if inherited.len() > super::classes::MAX_BINDINGS {
                            return Err(CommandError::Unavailable("Net assignment origin limit exceeded.".into()));
                        }
                    }
                }
            }
            record.class_bindings = inherited.into_iter().collect();
            assigned[index] = Some(record.id);
            result.insert(record.id, record);
        }
        let mut continuations = BTreeMap::<Uuid, Vec<Uuid>>::new();
        for (index, record) in &by_source {
            let branches = unique(targets[*index].iter().filter_map(|i| assigned[*i]));
            continuations.insert(record.id, branches.clone());
            let mut next = record.clone();
            next.primary.scope = old_scope;
            for anchor in &mut next.fallbacks {
                anchor.scope = old_scope;
            }
            next.state = if branches.is_empty() {
                State::Retired { snapshot: source_capture(&mut capture, before)?, reason: Retirement::Removed, successors: Vec::new() }
            } else if let Some(Some(chosen)) = chosen.get(&record.id) {
                if assigned[*chosen] == Some(record.id) {
                    continue;
                }
                let successor = assigned[*chosen].ok_or_else(|| CommandError::InvalidParams("Missing merge result identity.".into()))?;
                State::Retired { snapshot: source_capture(&mut capture, before)?, reason: Retirement::Merged, successors: vec![successor] }
            } else {
                State::Ambiguous { snapshot: source_capture(&mut capture, before)?, branches }
            };
            result.insert(record.id, next);
        }
        // Existing unresolved decisions survive later graph edits with current result references.
        // No surviving candidate means retirement; choosing among survivors remains explicit.
        for record in &registry.records {
            if let State::Ambiguous { snapshot, branches } = &record.state {
                let branches = unique(branches.iter().flat_map(|id| continuations.get(id).into_iter().flatten().copied()));
                let mut next = record.clone();
                next.state = if branches.is_empty() {
                    State::Retired { snapshot: source_capture(&mut capture, before)?, reason: Retirement::Removed, successors: Vec::new() }
                } else {
                    State::Ambiguous { snapshot: snapshot.clone(), branches }
                };
                result.insert(record.id, next);
            }
        }
        let next = Registry { records: result.into_values().collect() };
        if next == *self.net_registry {
            return Ok(());
        }
        self.net_registry = Arc::new(next);
        if !self.net_registry.records.is_empty() {
            if self.layout.is_none() {
                *self = self.with_board_folders(".").map_err(CommandError::InvalidParams)?;
            }
            if !self.has_sheet_occurrences() {
                *self = self.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
            }
            self.net_registry.validate(self).map_err(CommandError::InvalidParams)?;
        }
        Ok(())
    }
}
