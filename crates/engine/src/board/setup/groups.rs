//! Layer scopes derived from the one physical stack, with optional authored named memberships.
use super::{BoardSetup, CopperRole, Layer, LayerPurpose, MAX_COPPER, MAX_NAME, unique_id, valid_text};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Inspect current derived and user-defined groups without changing project state.
pub const INSPECT_LAYER_GROUPS: &str = "pcb.inspect_layer_groups";
/// Bound saved membership and per-edit copies to 64 groups of at most 64 layers each.
pub const MAX_GROUPS: usize = 64;

/// Authored copper membership, independent of display names and physical layer positions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserLayerGroup {
    pub id: Uuid,
    pub name: String,
    pub layers: Vec<Uuid>,
}

impl UserLayerGroup {
    /// Allocate an empty draft identity; the complete setup validates its edited name and members.
    pub fn new() -> Result<Self, crate::CommandError> {
        Ok(Self { id: crate::fresh_id()?, name: String::new(), layers: Vec::new() })
    }
}

#[cfg(test)]
mod tests;

/// One named membership resolved to current layer labels in physical order.
#[derive(Debug, Serialize)]
pub struct ResolvedUserGroup {
    pub id: Uuid,
    pub name: String,
    pub layers: Vec<Layer>,
}

/// Computed physical and role scopes; only the user memberships are saved on the stack.
#[derive(Debug, Serialize)]
pub struct LayerGroups {
    pub outer: Vec<Layer>,
    pub inner: Vec<Layer>,
    pub plane: Vec<Layer>,
    pub signal: Vec<Layer>,
    pub user: Vec<ResolvedUserGroup>,
}

impl BoardSetup {
    /// Resolve all groups from current strata. Mixed copper belongs to both role groups;
    /// an unspecified role belongs to neither. Every membership is in physical order.
    pub fn layer_groups(&self) -> LayerGroups {
        let copper: Vec<_> = self.copper_stack().collect();
        let layers = |ids: Vec<Uuid>| ids.into_iter().filter_map(|id| self.layers.iter().find(|l| l.id == id).cloned()).collect();
        let selected = |predicate: &dyn Fn(usize, &super::CopperFacts) -> bool| {
            layers(copper.iter().enumerate().filter(|(i, c)| predicate(*i, c)).map(|(_, c)| c.layer).collect())
        };
        LayerGroups {
            outer: selected(&|i, _| i == 0 || i + 1 == copper.len()),
            inner: selected(&|i, _| i > 0 && i + 1 < copper.len()),
            plane: selected(&|_, c| matches!(c.role, Some(CopperRole::Plane | CopperRole::Mixed))),
            signal: selected(&|_, c| matches!(c.role, Some(CopperRole::Signal | CopperRole::Mixed))),
            user: self
                .stackup
                .layer_groups
                .iter()
                .map(|g| ResolvedUserGroup { id: g.id, name: g.name.clone(), layers: layers(g.layers.clone()) })
                .collect(),
        }
    }

    /// Layer groups require their own saved format only when authored memberships exist.
    pub(crate) fn requires_group_format(&self) -> bool {
        !self.stackup.layer_groups.is_empty()
    }
}

/// Reject unknown or repeated members and identities, then canonicalize by physical order.
/// Empty memberships remain valid when removing the group's final copper layer.
pub(super) fn validate(setup: &mut super::Stackup, ids: &mut BTreeSet<Uuid>, copper: &BTreeSet<Uuid>) -> Result<(), String> {
    if setup.layer_groups.len() > MAX_GROUPS {
        return Err(format!("a stack supports at most {MAX_GROUPS} user layer groups"));
    }
    let positions: BTreeMap<_, _> = setup
        .strata
        .iter()
        .filter_map(|s| match s {
            super::Stratum::Copper { layer, .. } => Some(*layer),
            _ => None,
        })
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect();
    let mut names = BTreeSet::new();
    for group in &mut setup.layer_groups {
        unique_id(ids, group.id)?;
        valid_text("layer group name", &group.name)?;
        if !names.insert(group.name.to_lowercase()) {
            return Err("user layer group names must be unique without case".into());
        }
        let members: BTreeSet<_> = group.layers.iter().copied().collect();
        if group.layers.len() > MAX_COPPER || members.len() != group.layers.len() || !members.is_subset(copper) {
            return Err("user layer groups need distinct copper layer IDs from this stack, at most 64 per group".into());
        }
        group.layers.sort_by_key(|id| positions.get(id));
    }
    setup.layer_groups.sort_by_key(|g| g.id);
    Ok(())
}

/// Describe exactly the saved authored groups, never a second stored copy of derived groups.
pub(super) fn schema() -> Value {
    json!({"type":"array","maxItems":MAX_GROUPS,"items":{"type":"object","additionalProperties":false,"required":["id","name","layers"],"properties":{
        "id":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":MAX_NAME},
        "layers":{"type":"array","maxItems":MAX_COPPER,"uniqueItems":true,"items":{"type":"string","format":"uuid"}}
    }}})
}

/// Before dropping deleted copper, reject submitted lists that could become valid only by pruning.
fn prune_removed_members(layers: &mut Vec<Value>, removed: &BTreeSet<Uuid>) -> Result<(), crate::CommandError> {
    if layers.len() > MAX_COPPER {
        return Err(crate::CommandError::InvalidParams(format!("user layer groups support at most {MAX_COPPER} submitted members per group")));
    }
    let mut submitted = BTreeSet::new();
    for member in layers.iter() {
        let id = Uuid::deserialize(member).map_err(|error| crate::CommandError::InvalidParams(error.to_string()))?;
        if !submitted.insert(id) {
            return Err(crate::CommandError::InvalidParams("user layer groups need distinct submitted layer IDs".into()));
        }
    }
    layers.retain(|id| !id.as_str().and_then(|s| Uuid::parse_str(s).ok()).is_some_and(|id| removed.contains(&id)));
    Ok(())
}

/// Preview and Apply validate submitted memberships before removing deleted former copper IDs.
/// Invented references still fail the complete candidate validation.
pub(super) fn candidate(project: &crate::Project, value: &Value) -> Result<BoardSetup, crate::CommandError> {
    let mut value = value.clone();
    if let Some(before) = project.board_setup() {
        let current: BTreeSet<_> = value["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|l| l["purpose"] == "copper")
            .filter_map(|l| l["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()))
            .collect();
        let removed: BTreeSet<_> =
            before.layers.iter().filter(|l| l.purpose == LayerPurpose::Copper && !current.contains(&l.id)).map(|l| l.id).collect();
        if !removed.is_empty()
            && let Some(groups) = value.get_mut("stackup").and_then(|s| s.get_mut("layer_groups")).and_then(Value::as_array_mut)
        {
            for group in groups {
                if let Some(layers) = group.get_mut("layers").and_then(Value::as_array_mut) {
                    prune_removed_members(layers, &removed)?;
                }
            }
        }
    }
    serde_json::from_value(value).map_err(|e| crate::CommandError::InvalidParams(e.to_string()))
}
