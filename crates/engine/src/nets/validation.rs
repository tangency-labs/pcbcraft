//! Validate bounded persisted identity authority against the complete current electrical graph.

use super::{Anchor, AnchorKind, MAX_RECORDS, MAX_REGISTRY_BYTES, Registry, Retirement, ScopeKind, State, graph::Graph};
use crate::Project;
use std::collections::BTreeSet;

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
    pub(crate) fn validate(&self, project: &Project) -> Result<(), String> {
        if self.records.len() > MAX_RECORDS || (!self.records.is_empty() && !project.has_sheet_occurrences()) {
            return Err("Net registry requires adopted occurrences and a bounded record count.".into());
        }
        let ids: BTreeSet<_> = self.records.iter().map(|r| r.id).collect();
        if ids.len() != self.records.len() || ids.contains(&uuid::Uuid::nil()) || self.records.windows(2).any(|p| p[0].id >= p[1].id) {
            return Err("Net records require unique non-nil UUIDs in canonical order.".into());
        }
        let graph = Graph::build(project)?;
        let mut assigned = BTreeSet::new();
        for record in &self.records {
            if record.fallbacks.len() > 3
                || record.lineage.len() > MAX_RECORDS
                || record.anchors().collect::<BTreeSet<_>>().len() != record.fallbacks.len() + 1
            {
                return Err("Duplicate or excessive net anchors/lineage.".into());
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
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_REGISTRY_BYTES {
            return Err("Net registry exceeds its byte limit.".into());
        }
        Ok(())
    }
}

impl Project {
    /// Inspect persistent and revision-derived addresses without promotion, allocation or edits.
    /// Derived membership remains in the ordinary netlist; each row here supplies a typed key.
    pub fn inspect_net_registry(&self) -> Result<serde_json::Value, String> {
        let graph = Graph::build(self)?;
        let nets: Vec<_> = graph
            .networks
            .iter()
            .enumerate()
            .filter(|(_, n)| n.meaningful())
            .map(|(index, network)| {
                let id = self
                    .net_registry
                    .records
                    .iter()
                    .find(|r| matches!(r.state, State::Active {}) && graph.network(&r.primary) == Some(index))
                    .map(|r| r.id);
                serde_json::json!({"id":id,"anchor":network.anchors.first(),"names":network.names})
            })
            .collect();
        Ok(serde_json::json!({"scope":self.net_scope(),"records":self.net_registry.records,"nets":nets}))
    }
}
