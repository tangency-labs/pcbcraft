//! Validate bounded persisted identity authority against the complete current electrical graph.

use super::{Anchor, AnchorKind, MAX_RECORDS, MAX_REGISTRY_BYTES, Registry, Retirement, ScopeKind, State, graph::Graph};
use crate::{CommandError, Project};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, Weak},
};
use uuid::Uuid;

/// Evidence from a successful ordinary validation of the immutable, non-pair registry
/// inputs. Keeping weak identities prevents a proof from retaining obsolete project data.
#[derive(Clone, Debug)]
struct RecordsProof {
    records: Weak<Vec<super::Record>>,
    sheets: Weak<Vec<crate::sheets::SchematicSheet>>,
    units: Weak<BTreeMap<(u64, String), u64>>,
    components: Weak<BTreeMap<u64, crate::schematic::PartInstance>>,
    classes: Weak<crate::net_classes::Catalog>,
    project: Uuid,
    scope: super::Scope,
    circuit_use: Option<Uuid>,
}

impl RecordsProof {
    /// Pair rows, project names and storage paths are not inputs to record/graph validation.
    fn new(registry: &Registry, project: &Project) -> Self {
        Self {
            records: Arc::downgrade(&registry.records),
            sheets: Arc::downgrade(&project.sheets),
            units: Arc::downgrade(&project.units),
            components: Arc::downgrade(&project.components),
            classes: Arc::downgrade(&project.net_classes),
            project: project.id,
            scope: project.net_scope(),
            circuit_use: project.circuit_use,
        }
    }

    /// Compare the exact validation inputs; no graph traversal or content hashing.
    fn matches(&self, next: &Self) -> bool {
        self.records.ptr_eq(&next.records)
            && self.sheets.ptr_eq(&next.sheets)
            && self.units.ptr_eq(&next.units)
            && self.components.ptr_eq(&next.components)
            && self.classes.ptr_eq(&next.classes)
            && self.project == next.project
            && self.scope == next.scope
            && self.circuit_use == next.circuit_use
    }
}

/// A disposable proof, independently owned by each snapshot so history or background Save
/// cannot overwrite another snapshot's evidence. Access never holds the lock during graph work.
#[derive(Debug, Default)]
pub(crate) struct RecordsProofCache(Mutex<Option<RecordsProof>>);

impl Clone for RecordsProofCache {
    fn clone(&self) -> Self {
        Self(Mutex::new(self.0.lock().ok().and_then(|proof| proof.clone())))
    }
}

impl RecordsProofCache {
    /// Check if the retained proof applies to the exact immutable record/graph ingredients.
    fn contains(&self, proof: &RecordsProof) -> bool {
        self.0.lock().is_ok_and(|stored| stored.as_ref().is_some_and(|stored| stored.matches(proof)))
    }

    /// Remember only a successful full validation.
    fn remember(&self, proof: RecordsProof) {
        if let Ok(mut stored) = self.0.lock() {
            *stored = Some(proof);
        }
    }
}

/// Live/storage anchors use the sole owner; historical lineage may additionally retain the
/// schematic scope preceding first-board creation. Unsupported system/junction keys are refused.
fn anchor_owner(anchor: &Anchor, project: &Project, historical: bool) -> Result<(), String> {
    anchor.validate_form()?;
    if anchor.project != project.id || anchor.kind == AnchorKind::Junction || anchor.scope.kind == ScopeKind::System {
        return Err("Foreign or unsupported registry anchor.".into());
    }
    if anchor.scope != project.net_scope() && !(historical && anchor.scope.kind == ScopeKind::Schematic && anchor.scope.id == project.id) {
        return Err("Registry anchor names a foreign electrical owner.".into());
    }
    Ok(())
}

impl Registry {
    /// Reject malformed records, historical references, duplicate live authorities and dangling
    /// choices before adoption. Inactive source anchors are historical, never live references.
    /// Empty registries need no graph; nonempty registries require adopted occurrences.
    pub(crate) fn validate(&self, project: &Project) -> Result<(), String> {
        if self.records.len() > MAX_RECORDS || (!self.records.is_empty() && !project.has_sheet_occurrences()) {
            return Err("Net registry requires adopted occurrences and a bounded record count.".into());
        }
        self.validate_pairs()?;
        if self.records.is_empty() {
            return Ok(());
        }
        self.validate_records(project)?;
        self.validate_byte_limit().map_err(|error| error.to_string())
    }

    /// Pair preparation has already checked the changed pair rows and the compact byte cap.
    /// Reuse the exact non-pair proof when it survives a pair-only clone; an unproven source
    /// still takes the full validation path rather than waiving any registry check.
    pub(crate) fn validate_pair_capacity_candidate(&self, project: &Project) -> Result<(), String> {
        if self.records.len() > MAX_RECORDS || (!self.records.is_empty() && !project.has_sheet_occurrences()) {
            return Err("Net registry requires adopted occurrences and a bounded record count.".into());
        }
        if self.records.is_empty() || self.has_records_proof(project) {
            return Ok(());
        }
        self.validate_records(project)
    }

    /// Check for evidence that this exact non-pair graph/registry input was validated.
    pub(super) fn has_records_proof(&self, project: &Project) -> bool {
        project.net_records_proof.contains(&RecordsProof::new(self, project))
    }

    /// Validate the complete graph and every non-pair record against it. Pair relationships
    /// cannot change the record anchors, number allocation, scope or owning catalogs.
    fn validate_records(&self, project: &Project) -> Result<(), String> {
        self.validate_records_against(project, None)
    }

    /// Retain exact validation from a graph already built by a transaction on this snapshot.
    pub(super) fn validate_records_against(&self, project: &Project, graph: Option<&Graph>) -> Result<(), String> {
        let ids: BTreeSet<_> = self.records.iter().map(|r| r.id).collect();
        if ids.len() != self.records.len() || ids.contains(&uuid::Uuid::nil()) || self.records.windows(2).any(|p| p[0].id >= p[1].id) {
            return Err("Net records require unique non-nil UUIDs in canonical order.".into());
        }
        let mut numbers = BTreeSet::new();
        if self.records.iter().filter_map(|record| record.generated_number).any(|number| number == 0 || !numbers.insert(number)) {
            return Err("Generated net numbers must be positive and unique, including historical records.".into());
        }
        let owned_graph;
        let graph = if let Some(graph) = graph {
            graph
        } else {
            owned_graph = Graph::build(project)?;
            &owned_graph
        };
        let mut assigned = BTreeSet::new();
        for record in self.records.as_slice() {
            if record.fallbacks.len() > 3
                || record.lineage.len() > MAX_RECORDS
                || record.anchors().collect::<BTreeSet<_>>().len() != record.fallbacks.len() + 1
            {
                return Err("Duplicate or excessive net anchors/lineage.".into());
            }
            if record.class_bindings.len() > super::classes::MAX_BINDINGS
                || record.class_bindings.windows(2).any(|pair| pair[0] >= pair[1])
                || record.class_bindings.iter().any(|binding| project.net_classes.get(binding.class).is_none() || !ids.contains(&binding.origin))
            {
                return Err("Invalid, dangling, duplicate or excessive net class bindings.".into());
            }
            for anchor in record.anchors() {
                anchor_owner(anchor, project, !matches!(record.state, State::Active {}))?;
            }
            let mut lineage = BTreeSet::new();
            for source in &record.lineage {
                source.snapshot.validate()?;
                anchor_owner(&source.anchor, project, true)?;
                if source.net.is_some_and(|id| id == record.id || !ids.contains(&id))
                    || !lineage.insert((source.snapshot.capture_id, source.net, &source.anchor))
                {
                    return Err("Invalid or duplicate snapshot-qualified net lineage.".into());
                }
            }
            let historical = match &record.state {
                State::Active {} => {
                    let index = graph.network(&record.primary).ok_or("Missing active net primary.")?;
                    if !graph.networks[index].meaningful()
                        || !assigned.insert(index)
                        || record.fallbacks.iter().any(|a| graph.network(a) != Some(index))
                    {
                        return Err("Active net anchors do not uniquely identify one meaningful network.".into());
                    }
                    None
                }
                State::Ambiguous { snapshot, branches } => {
                    if branches.is_empty()
                        || branches.len() > MAX_RECORDS
                        || branches.windows(2).any(|p| p[0] >= p[1])
                        || branches.iter().any(|id| *id == record.id || !self.record(*id).is_some_and(|r| matches!(r.state, State::Active {})))
                    {
                        return Err("Ambiguity branches require distinct current net identities.".into());
                    }
                    Some(snapshot)
                }
                State::Retired { snapshot, reason, successors } => {
                    if successors.len() > MAX_RECORDS
                        || successors.windows(2).any(|p| p[0] >= p[1])
                        || successors.iter().any(|id| *id == record.id || !ids.contains(id))
                        || (*reason == Retirement::Removed) != successors.is_empty()
                    {
                        return Err("Invalid retired net disposition.".into());
                    }
                    Some(snapshot)
                }
            };
            if let Some(snapshot) = historical {
                snapshot.validate()?;
            }
        }
        super::numbering::plan(self, &graph.networks)?;
        project.net_records_proof.remember(RecordsProof::new(self, project));
        Ok(())
    }

    /// Check the native serialized limit without rebuilding the graph, distinguishing a
    /// fixed cap from other serialization failures for pair-only command callers.
    pub(super) fn validate_byte_limit(&self) -> Result<(), CommandError> {
        if serde_json::to_vec(self).map_err(|error| CommandError::InvalidParams(error.to_string()))?.len() > MAX_REGISTRY_BYTES {
            return Err(CommandError::CapExceeded("Net registry exceeds its byte limit.".into()));
        }
        Ok(())
    }
}

impl Project {
    /// Inspect persistent and revision-derived addresses without promotion, allocation or edits.
    /// Derived membership remains in the ordinary netlist; each row here supplies a typed key.
    /// Explicit differential relationships appear only when stored.
    pub fn inspect_net_registry(&self) -> Result<serde_json::Value, String> {
        let graph = Graph::build(self)?;
        let electrical = self.electrical_connectivity();
        let mut active_ids = vec![None; graph.networks.len()];
        for record in self.net_registry.records.as_slice() {
            if matches!(record.state, State::Active {})
                && let Some(index) = graph.network(&record.primary)
            {
                active_ids[index].get_or_insert(record.id);
            }
        }
        let nets: Vec<_> = graph
            .networks
            .iter()
            .enumerate()
            .filter(|(_, n)| n.meaningful())
            .map(|(index, network)| {
                let id = active_ids[index];
                let display_name = network
                    .anchors
                    .iter()
                    .find_map(|anchor| electrical.net_index_for_endpoint(self.anchor_endpoint(anchor)?).map(|index| electrical.net_name(index)))
                    .unwrap_or_else(|| "Unresolved net".into());
                serde_json::json!({"id":id,"anchor":network.anchors.first(),"names":network.names,"name":display_name})
            })
            .collect();
        let mut result = serde_json::json!({"scope":self.net_scope(),"records":self.net_registry.records,"nets":nets});
        if self.net_registry.has_pairs() {
            result["pairs"] = serde_json::to_value(&self.net_registry.pairs).map_err(|error| error.to_string())?;
        }
        Ok(result)
    }

    /// Return the active persistent ID of each row of [`Project::electrical_connectivity`], by
    /// row index; rows without one are `None`. Reads only, like [`Self::inspect_net_registry`].
    pub(crate) fn persistent_net_ids(&self) -> Result<Vec<Option<uuid::Uuid>>, String> {
        let electrical = self.electrical_connectivity();
        let mut ids = vec![None; electrical.nets.len()];
        if self.net_registry.records.is_empty() {
            return Ok(ids);
        }
        let graph = Graph::build(self)?;
        for record in self.net_registry.records.as_slice() {
            if !matches!(record.state, State::Active {}) {
                continue;
            }
            let Some(network) = graph.network(&record.primary) else { continue };
            let row = graph.networks[network].anchors.iter().find_map(|anchor| electrical.net_index_for_endpoint(self.anchor_endpoint(anchor)?));
            if let Some(slot) = row.and_then(|row| ids.get_mut(row)) {
                slot.get_or_insert(record.id);
            }
        }
        Ok(ids)
    }

    /// The current schematic endpoint of a live anchor; junction anchors have none.
    fn anchor_endpoint(&self, anchor: &Anchor) -> Option<crate::WireEndpoint> {
        match anchor.kind {
            AnchorKind::Terminal => self.component_terminal(anchor.object, &anchor.terminal).map(crate::WireEndpoint::Terminal),
            AnchorKind::Label => Some(crate::WireEndpoint::NetLabel(anchor.object)),
            AnchorKind::Port => Some(crate::WireEndpoint::Label(anchor.object)),
            AnchorKind::Junction => None,
        }
    }
}
