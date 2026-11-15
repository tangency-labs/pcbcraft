//! Shared revision-checked authored promotion and explicit ambiguity resolution.

use super::{MAX_RECORDS, Registry, Retirement, State, graph::Graph, reconcile::allocate};
use crate::{CommandError, Project, Session};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

/// Promote a meaningful derived network through a terminal, named label or hierarchical port.
pub const PROMOTE: &str = "nets.promote";
/// Assign an unresolved source identity to an explicitly addressed current candidate branch.
pub const RESOLVE_AMBIGUITY: &str = "nets.resolve_ambiguity";

/// Validate the optional physical owner and resolve a current endpoint in explicit sheet context.
fn target(project: &Project, params: &Value) -> Result<(Graph, super::Anchor), CommandError> {
    if params.get("board").is_some() {
        project.resolve_board(params["board"].as_str())?;
    }
    let sheet = project.resolve_sheet(params["sheet"].as_str())?;
    let graph = Graph::build(project).map_err(CommandError::InvalidParams)?;
    let anchor = graph.target(sheet, &params["anchor"])?;
    Ok((graph, anchor))
}

impl Session {
    /// Promote once without changing connectivity. Existing identities are no-ops; bare pins,
    /// orphan stubs, stale revisions and foreign/missing endpoints publish no state.
    pub(crate) fn promote_net(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let (graph, anchor) = target(original, params)?;
        let index = graph.network(&anchor).ok_or_else(|| CommandError::InvalidParams("Unknown net anchor.".into()))?;
        if original.net_registry.records.iter().any(|r| matches!(r.state, State::Active {}) && graph.network(&r.primary) == Some(index)) {
            return Ok(());
        }
        let mut project =
            if original.layout.is_none() { original.with_board_folders(".").map_err(CommandError::InvalidParams)? } else { original.clone() };
        if !project.has_sheet_occurrences() {
            project = project.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
        }
        let mut registry = (*project.net_registry).clone();
        let mut ids = registry.records.iter().map(|r| r.id).collect();
        let record = graph.networks[index].record(allocate(&mut ids)?, Some(&anchor)).map_err(CommandError::InvalidParams)?;
        registry.records.push(record);
        registry.records.sort_by_key(|r| r.id);
        project.net_registry = Arc::new(registry);
        project.net_registry.validate(&project).map_err(CommandError::InvalidParams)?;
        self.change(project)
    }

    /// Resolve a stored ambiguity once, preserving its UUID and retiring the provisional branch
    /// identity. Other open decisions follow the resulting live ID; Undo restores the whole choice.
    pub(crate) fn resolve_net_ambiguity(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let id = params["net"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| CommandError::InvalidParams("Expected an ambiguous net UUID.".into()))?;
        let source = original.net_registry.record(id).ok_or_else(|| CommandError::InvalidParams("Unknown persistent net UUID.".into()))?;
        let State::Ambiguous { branches, .. } = &source.state else {
            return Err(CommandError::InvalidParams("Net identity is not ambiguous.".into()));
        };
        let (graph, anchor) = target(original, params)?;
        let index = graph.network(&anchor).ok_or_else(|| CommandError::InvalidParams("Unknown resolution branch.".into()))?;
        let selected = original
            .net_registry
            .records
            .iter()
            .find(|r| branches.contains(&r.id) && matches!(r.state, State::Active {}) && graph.network(&r.primary) == Some(index))
            .ok_or_else(|| CommandError::InvalidParams("The selected network is not a current candidate branch.".into()))?;
        let mut project = original.clone();
        let snapshot = crate::board::update::capture_source(original)?;
        let mut records: BTreeMap<_, _> = original.net_registry.records.iter().map(|r| (r.id, r.clone())).collect();
        let mut restored = source.clone();
        restored.primary = selected.primary.clone();
        restored.fallbacks = selected.fallbacks.clone();
        restored.state = State::Active {};
        restored.class_bindings = selected.class_bindings.clone();
        records.insert(id, restored);
        let mut retired = selected.clone();
        retired.state = State::Retired { snapshot, reason: Retirement::Resolution, successors: vec![id] };
        records.insert(retired.id, retired);
        for record in records.values_mut() {
            if let State::Ambiguous { branches, .. } = &mut record.state {
                *branches =
                    branches.iter().map(|branch| if *branch == selected.id { id } else { *branch }).collect::<BTreeSet<_>>().into_iter().collect();
            }
        }
        if records.len() > MAX_RECORDS {
            return Err(CommandError::Unavailable("Net identity registry is full.".into()));
        }
        project.net_registry = Arc::new(Registry { records: records.into_values().collect() });
        project.net_registry.validate(&project).map_err(CommandError::InvalidParams)?;
        self.change(project)
    }
}
