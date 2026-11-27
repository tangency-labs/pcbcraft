//! Transactional continuation evidence comes from frozen before/after graphs, never saved members.

use super::{Anchor, Appearance, Lineage, MAX_RECORDS, Record, Registry, Retirement, Scope, State, graph::Graph};
use crate::{CommandError, Project, Terminal, WireEndpoint, board::update::SourceCapture};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

/// Allocate one net at the current record count. The collision set also reserves pair UUIDs,
/// but those do not consume net-record capacity.
pub(super) fn allocate(net_count: usize, ids: &mut BTreeSet<Uuid>) -> Result<Uuid, CommandError> {
    if net_count >= MAX_RECORDS {
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

/// Choose the drawing appearance a rebuilt successor inherits from its contributing sources.
///
/// Appearance is net-owned presentation intent, so a successor keeps the effective override of
/// any source it merges or splits from, even after that source retires: `wire_appearances` reads
/// only active records, and leaving a fresh successor unset would silently drop the override.
/// A single source override is inherited unchanged. Conflicting overrides resolve to the source
/// with the lowest UUID, matching the UUID-sorted registry and `unique()` ordering so the result
/// never depends on graph traversal or authoring order. Sources without an override contribute
/// nothing, leaving an unstyled net unstyled. Retired records are not consulted for their state
/// and always keep their own saved appearance as historical evidence.
fn inherited_appearance(parents: &[usize], by_source: &BTreeMap<usize, Record>) -> Option<Appearance> {
    parents
        .iter()
        .filter_map(|source| by_source.get(source))
        .filter(|record| record.appearance.is_some())
        .min_by_key(|record| record.id)
        .and_then(|record| record.appearance)
}

/// Preserve source-qualified identity evidence even when first-board creation changes live scope.
fn source_anchor(anchor: &Anchor, scope: Scope) -> Anchor {
    Anchor { scope, ..anchor.clone() }
}

/// Persist the exact numbers previously shown by a legacy graph before an edit can remove
/// its naming anchors. Current explicit records keep their IDs, primary choices and bindings.
fn adopt_numbers(registry: &mut Registry, graph: &Graph, plan: &super::numbering::Plan) -> Result<(), CommandError> {
    let mut ids = registry.reserved_ids();
    let mut last = plan.records.values().copied().chain(plan.networks.iter().flatten().copied()).max().unwrap_or(0);
    for record in &mut registry.records {
        if record.generated_number.is_none() {
            record.generated_number =
                Some(match plan.records.get(&record.id).copied().or_else(|| graph.network(&record.primary).and_then(|index| plan.networks[index])) {
                    Some(number) => number,
                    None => super::numbering::allocate(&mut last).map_err(CommandError::InvalidParams)?,
                });
        }
    }
    let existing: BTreeSet<_> = registry
        .records
        .iter()
        .filter(|record| matches!(record.state, State::Active {}))
        .filter_map(|record| graph.network(&record.primary))
        .collect();
    for (index, network) in graph.networks.iter().enumerate().filter(|(_, network)| network.meaningful()) {
        if !existing.contains(&index) {
            let mut record = network.record(allocate(registry.records.len(), &mut ids)?, None).map_err(CommandError::InvalidParams)?;
            record.generated_number = plan.networks[index];
            registry.records.push(record);
        }
    }
    registry.records.sort_by_key(|record| record.id);
    Ok(())
}

impl Project {
    /// Validate a unit transfer's exact electrical continuation, including connected appearances
    /// of shared pins. Unlike an ordinary wire edit, a transfer may replace every anchor of a
    /// net; identity follows the verified endpoint correspondence, not the old owner number.
    pub(crate) fn rekey_nets(&mut self, before: &Project, pins: &BTreeMap<Terminal, Terminal>) -> Result<(), CommandError> {
        let old = Graph::build(before).map_err(CommandError::InvalidParams)?;
        let new = Graph::build(self).map_err(CommandError::InvalidParams)?;
        let remap = |endpoint| match endpoint {
            WireEndpoint::Terminal(pin) => WireEndpoint::Terminal(pins.get(&pin).copied().unwrap_or(pin)),
            _ => endpoint,
        };
        let changed = || CommandError::InvalidParams("Completing these packages would change connectivity, including shared terminals.".into());
        if old.endpoints.len() != new.endpoints.len() || old.networks.len() != new.networks.len() {
            return Err(changed());
        }
        let mut forward = BTreeMap::new();
        let mut reverse = BTreeMap::new();
        let mut anchors = BTreeMap::new();
        for (&(sheet, endpoint), source) in &old.endpoints {
            let target = new.endpoints.get(&(sheet, remap(endpoint))).ok_or_else(changed)?;
            let (old_net, new_net) = (old.by_anchor[source], new.by_anchor[target]);
            if forward.insert(old_net, new_net).is_some_and(|previous| previous != new_net)
                || reverse.insert(new_net, old_net).is_some_and(|previous| previous != old_net)
                || anchors.insert(source, target).is_some_and(|previous| previous != target)
            {
                return Err(changed());
            }
        }
        for (source, target) in forward {
            let (source, target) = (&old.networks[source], &new.networks[target]);
            if source.meaningful() != target.meaningful() || source.names != target.names {
                return Err(changed());
            }
        }
        if anchors.iter().all(|(source, target)| *source == *target) {
            return Ok(());
        }
        before.net_registry.validate(before).map_err(CommandError::InvalidParams)?;
        let mut registry = (*before.net_registry).clone();
        let numbered: BTreeSet<_> = registry
            .records
            .iter()
            .filter(|record| matches!(record.state, State::Active {}) && record.generated_number.is_some())
            .filter_map(|record| old.network(&record.primary))
            .collect();
        if old.networks.iter().enumerate().any(|(index, network)| network.meaningful() && !numbered.contains(&index)) {
            return Err(CommandError::InvalidParams("Package completion requires persisted net identities and numbers.".into()));
        }
        for record in &mut registry.records {
            if matches!(record.state, State::Active {}) {
                record.primary = (*anchors.get(&record.primary).ok_or_else(changed)?).clone();
                let mut seen = BTreeSet::from([record.primary.clone()]);
                for fallback in &mut record.fallbacks {
                    *fallback = (*anchors.get(fallback).ok_or_else(changed)?).clone();
                }
                record.fallbacks.retain(|anchor| seen.insert(anchor.clone()));
            }
        }
        if registry != *self.net_registry {
            self.net_registry = Arc::new(registry);
            self.invalidate_net_presentation();
            self.net_registry.validate(self).map_err(CommandError::InvalidParams)?;
        }
        Ok(())
    }

    /// Reconcile one canonical candidate before history publication. Non-anchor changes keep exact
    /// registry bytes; allocation/validation failure leaves the owning session untouched.
    /// Rebuilt records inherit the union of active source parents' class bindings, deduplicated
    /// by class and origin. Inherited classes must exist in the catalog, and meaningful splits
    /// require every inherited property to be shareable. Missing classes, unsupported split
    /// inheritance or a union above `MAX_BINDINGS` refuse the edit before session publication.
    /// Stored pair members and review intent are preserved, not retargeted or evaluated here.
    /// An explicit pair edit combined with a topology change is refused rather than discarded.
    /// Newly adopted registries and changed registries on unchanged topology also validate,
    /// even when they contain pairs but no net records. Rebuilt successors carry forward the
    /// effective appearance of their contributing sources; conflicting overrides resolve to the
    /// lowest source UUID (see [`inherited_appearance`]).
    pub(crate) fn reconcile_nets(&mut self, before: Option<&Project>) -> Result<(), CommandError> {
        let Some(before) = before.filter(|before| before.same_document(self)) else {
            return self.net_registry.validate(self).map_err(CommandError::InvalidParams);
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
        #[cfg(test)]
        crate::annotation::tests::count_work(|work| work.reconcile_graphs += 1);
        let mut old = Graph::build(before).map_err(CommandError::InvalidParams)?;
        #[cfg(test)]
        crate::annotation::tests::count_work(|work| work.reconcile_graphs += 1);
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
            let identity_changed = old_scope != scope || explicit != *before.net_registry;
            if identity_changed {
                let plan = super::numbering::plan(&registry, &old.networks).map_err(CommandError::InvalidParams)?;
                adopt_numbers(&mut explicit, &after, &plan)?;
            }
            if explicit != *self.net_registry {
                self.net_registry = Arc::new(explicit);
            }
            if identity_changed {
                self.invalidate_net_presentation();
            }
            self.net_registry.validate(self).map_err(CommandError::InvalidParams)?;
            return Ok(());
        }
        if !Arc::ptr_eq(&self.net_registry.pairs, &registry.pairs) && self.net_registry.pairs != registry.pairs {
            return Err(CommandError::InvalidParams("Differential pair edits cannot accompany net topology changes.".into()));
        }
        before.net_registry.validate(before).map_err(CommandError::InvalidParams)?;
        let plan = super::numbering::plan(&registry, &old.networks).map_err(CommandError::InvalidParams)?;
        adopt_numbers(&mut registry, &old, &plan)?;
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
        let mut ids = registry.reserved_ids();
        let mut assigned = vec![None; after.networks.len()];
        let mut capture = None;
        let mut last_number = registry.records.iter().filter_map(|record| record.generated_number).max().unwrap_or(0);
        for (index, network) in after.networks.iter().enumerate().filter(|(_, n)| n.meaningful()) {
            let parents = &sources[index];
            let persistent = parents.iter().any(|source| by_source.contains_key(source));
            let retained = if parents.len() == 1 {
                parents.first().and_then(|source| by_source.get(source)).filter(|r| chosen.get(&r.id) == Some(&Some(index)))
            } else {
                None
            };
            let mut record = if let Some(retained) = retained {
                network.retained(retained).map_err(CommandError::InvalidParams)?
            } else {
                let mut record = network.record(allocate(result.len(), &mut ids)?, None).map_err(CommandError::InvalidParams)?;
                record.generated_number = Some(super::numbering::allocate(&mut last_number).map_err(CommandError::InvalidParams)?);
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
                            net: by_source.get(source).filter(|r| before.net_registry.record(r.id).is_some()).map(|r| r.id),
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
                            return Err(CommandError::CapExceeded("Net assignment origin limit exceeded.".into()));
                        }
                    }
                }
            }
            record.class_bindings = inherited.into_iter().collect();
            record.appearance = inherited_appearance(parents, &by_source);
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
        let next = Registry { records: result.into_values().collect(), pairs: registry.pairs };
        if next == *self.net_registry {
            return Ok(());
        }
        self.net_registry = Arc::new(next);
        self.invalidate_net_presentation();
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

    /// Number adoption changes presentation without changing electrical topology. Drop both
    /// the project graph and sheet projections so readers cannot keep a provisional name.
    fn invalidate_net_presentation(&mut self) {
        self.named_graph = Arc::default();
        for sheet in Arc::make_mut(&mut self.sheets) {
            sheet.connectivity = Arc::default();
        }
    }
}
