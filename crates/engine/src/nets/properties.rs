//! Apply a shared appearance and class-membership patch to several logical nets atomically.

use super::{Appearance, ClassBinding, ClassTarget, State, appearance::Patch, graph::Graph, reconcile::allocate};
use crate::{CommandError, Project, Session, net_classes};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

/// Apply partial appearance changes and class additions/removals to multiple nets.
pub const SET_PROPERTIES: &str = "nets.set_properties";
/// Bound bulk mutation and inspection work independently of the schematic size.
pub const MAX_PROPERTY_TARGETS: usize = 64;

/// Current authored values for one net, without expanding its effective rule hierarchy.
#[derive(Debug, Default)]
pub struct PropertyValues {
    pub appearance: Option<Appearance>,
    pub classes: BTreeSet<Uuid>,
}

#[derive(Debug)]
struct Changes {
    appearance: Option<Patch>,
    add: Vec<Uuid>,
    remove: Vec<Uuid>,
}

impl Changes {
    /// Omitted fields leave each net's value intact. A class can have only one requested action.
    fn parse(value: &Value) -> Result<Self, CommandError> {
        let invalid = || CommandError::InvalidParams("Provide appearance, add_classes or remove_classes.".into());
        let fields = value.as_object().filter(|fields| !fields.is_empty()).ok_or_else(invalid)?;
        if fields.keys().any(|key| !matches!(key.as_str(), "appearance" | "add_classes" | "remove_classes")) {
            return Err(invalid());
        }
        let classes = |key| fields.get(key).map(net_classes::parse_set).transpose().map(Option::unwrap_or_default);
        let add = classes("add_classes")?;
        let remove = classes("remove_classes")?;
        if add.iter().any(|id| remove.contains(id)) {
            return Err(CommandError::InvalidParams("A class cannot be both added and removed.".into()));
        }
        Ok(Self { appearance: fields.get("appearance").map(Patch::parse).transpose()?, add, remove })
    }
}

/// Parse distinct typed targets before resolving electrical aliases.
fn parse_targets(value: &Value) -> Result<Vec<ClassTarget>, CommandError> {
    let values = value
        .as_array()
        .filter(|items| (1..=MAX_PROPERTY_TARGETS).contains(&items.len()))
        .ok_or_else(|| CommandError::InvalidParams(format!("Choose 1 to {MAX_PROPERTY_TARGETS} net targets.")))?;
    let mut targets = Vec::with_capacity(values.len());
    for value in values {
        let target = ClassTarget::parse(value)?;
        if targets.contains(&target) {
            return Err(CommandError::InvalidParams("Net targets must be distinct.".into()));
        }
        targets.push(target);
    }
    Ok(targets)
}

pub(crate) fn property_targets_schema() -> Value {
    json!({"type":"array", "minItems":1, "maxItems":MAX_PROPERTY_TARGETS, "uniqueItems":true, "items":super::class_target_schema()})
}

pub(crate) fn property_changes_schema() -> Value {
    json!({"type":"object", "additionalProperties":false, "minProperties":1, "properties":{
        "appearance":super::appearance_param_schema(), "add_classes":net_classes::set_schema(), "remove_classes":net_classes::set_schema()
    }})
}

pub(crate) fn validate_property_targets(value: &Value) -> Result<(), &'static str> {
    parse_targets(value).map(|_| ()).map_err(|_| "must contain 1 to 64 distinct scoped net targets")
}

pub(crate) fn validate_property_changes(value: &Value) -> Result<(), &'static str> {
    Changes::parse(value).map(|_| ()).map_err(|_| "must contain valid appearance or disjoint class changes")
}

/// Resolve each target on one graph and return its current network and saved identity.
fn resolve(project: &Project, graph: &Graph, targets: &[ClassTarget]) -> Result<Vec<(usize, Option<Uuid>)>, CommandError> {
    let scope = project.net_scope();
    let active: BTreeMap<_, _> = project
        .net_registry
        .records
        .iter()
        .filter(|record| matches!(record.state, State::Active {}))
        .filter_map(|record| Some((graph.network(&record.primary)?, record.id)))
        .collect();
    let mut seen = BTreeSet::new();
    targets
        .iter()
        .map(|target| {
            let anchor = match target {
                ClassTarget::Persistent { project: owner, scope: target_scope, net } if *owner == project.id && *target_scope == scope => {
                    &project
                        .net_registry
                        .record(*net)
                        .filter(|record| matches!(record.state, State::Active {}))
                        .ok_or_else(|| CommandError::NotFound("Unknown or inactive net identity.".into()))?
                        .primary
                }
                ClassTarget::Derived { anchor } if anchor.project == project.id && anchor.scope == scope => anchor,
                _ => return Err(CommandError::InvalidParams("Foreign net owner.".into())),
            };
            let network = graph
                .network(anchor)
                .filter(|index| graph.networks[*index].meaningful())
                .ok_or_else(|| CommandError::InvalidParams("Choose current meaningful nets.".into()))?;
            if !seen.insert(network) {
                return Err(CommandError::InvalidParams("Targets must resolve to distinct nets.".into()));
            }
            Ok((network, active.get(&network).copied()))
        })
        .collect()
}

impl Project {
    /// Read authored values in target order without promoting derived nets or changing history.
    pub fn net_property_values(&self, targets: &[ClassTarget]) -> Result<Vec<PropertyValues>, CommandError> {
        let graph = Graph::build(self).map_err(CommandError::InvalidParams)?;
        resolve(self, &graph, targets).map(|resolved| {
            resolved
                .into_iter()
                .map(|(_, id)| {
                    id.and_then(|id| self.net_registry.record(id)).map_or_else(PropertyValues::default, |record| PropertyValues {
                        appearance: record.appearance,
                        classes: record.class_bindings.iter().map(|binding| binding.class).collect(),
                    })
                })
                .collect()
        })
    }
}

impl Session {
    /// Validate all requested nets before publishing one shared revision for the complete edit.
    pub(crate) fn set_net_properties(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let targets = parse_targets(&params["targets"])?;
        let changes = Changes::parse(&params["changes"])?;
        if changes.add.iter().chain(&changes.remove).any(|id| original.net_classes.get(*id).is_none()) {
            return Err(CommandError::NotFound("Unknown net class.".into()));
        }
        let graph = Graph::build(original).map_err(CommandError::InvalidParams)?;
        let resolved = resolve(original, &graph, &targets)?;
        let mut registry = (*original.net_registry).clone();
        let mut ids = registry.reserved_ids();
        let mut changed = false;
        for (target, (network, existing)) in targets.iter().zip(resolved) {
            let saved = existing.and_then(|id| original.net_registry.record(id));
            let saved_appearance = saved.and_then(|record| record.appearance);
            let appearance = changes.appearance.as_ref().map_or(saved_appearance, |patch| patch.apply(saved_appearance));
            let mut bindings = saved.map_or_else(Vec::new, |record| record.class_bindings.clone());
            bindings.retain(|binding| !changes.remove.contains(&binding.class));
            let additions: Vec<_> = changes.add.iter().filter(|id| !bindings.iter().any(|binding| binding.class == **id)).collect();
            if appearance == saved_appearance && additions.is_empty() && saved.is_none_or(|record| record.class_bindings == bindings) {
                continue;
            }
            let record = if let Some(id) = existing {
                registry.records.iter_mut().find(|record| record.id == id).ok_or_else(|| CommandError::NotFound("Missing net identity.".into()))?
            } else {
                let ClassTarget::Derived { anchor } = target else {
                    return Err(CommandError::InvalidParams("A derived net needs a current anchor.".into()));
                };
                let id = allocate(registry.records.len(), &mut ids)?;
                registry.records.push_mut(graph.networks[network].record(id, Some(anchor)).map_err(CommandError::InvalidParams)?)
            };
            bindings.extend(additions.into_iter().map(|class| ClassBinding { class: *class, origin: record.id }));
            bindings.sort();
            record.class_bindings = bindings;
            record.appearance = appearance;
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        let mut candidate =
            if original.layout.is_none() { original.with_board_folders(".").map_err(CommandError::InvalidParams)? } else { original.clone() };
        if !candidate.has_sheet_occurrences() {
            candidate = candidate.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
        }
        registry.records.sort_by_key(|record| record.id);
        candidate.net_registry = Arc::new(registry);
        candidate.net_registry.validate(&candidate).map_err(CommandError::InvalidParams)?;
        self.change(candidate)
    }
}
