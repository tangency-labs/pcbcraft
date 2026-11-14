//! Scoped dimensional assignments share registry ownership and retain their originating net IDs.
use super::{Anchor, Registry, Scope, State, graph::Graph, reconcile::allocate};
use crate::{CommandError, Project, Session, net_classes};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

/// Replace the complete classification set on one current net, promoting eligible derived intent.
pub const SET_CLASSES: &str = "nets.set_classes";
/// Inspect current or historical assignments and their effective/conflicting dimensional intent.
pub const INSPECT_CLASSES: &str = "nets.inspect_classes";
/// Bound accumulated split/merge origins independently of connectivity membership.
pub const MAX_BINDINGS: usize = super::MAX_RECORDS;

/// A classification with its original assigning net, in this registry's project/instance home.
/// Split/merge copies retain that origin; explicit assignment uses the current net identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassBinding {
    pub class: Uuid,
    pub origin: Uuid,
}

/// Persisted identities are always qualified by project and logical electrical owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetAddress {
    pub project: Uuid,
    pub scope: Scope,
    pub net: Uuid,
}

/// A revision-checked target is either a scoped saved identity or a current typed derived anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClassTarget {
    Persistent { project: Uuid, scope: Scope, net: Uuid },
    Derived { anchor: Anchor },
}
impl ClassTarget {
    /// Validate the typed envelope independently of current project membership.
    pub(crate) fn parse(value: &Value) -> Result<Self, CommandError> {
        let target: Self = serde_json::from_value(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        match &target {
            Self::Persistent { project, scope, net } if project.is_nil() || scope.id.is_nil() || net.is_nil() => {
                return Err(CommandError::InvalidParams("Net addresses require non-nil project, scope and net identities.".into()));
            }
            Self::Derived { anchor } => anchor.validate_form().map_err(CommandError::InvalidParams)?,
            _ => {}
        }
        Ok(target)
    }
}

/// Discovery exposes exact structured addresses, never names or unsaved membership arrays.
pub(crate) fn target_schema() -> Value {
    let uuid = json!({"type":"string","format":"uuid"});
    let scope =
        json!({"type":"object","additionalProperties":false,"required":["kind","id"],"properties":{"kind":{"enum":["schematic","board"]},"id":uuid}});
    json!({"oneOf":[
        {"type":"object","additionalProperties":false,"required":["kind","project","scope","net"],"properties":{"kind":{"const":"persistent"},"project":uuid,"scope":scope,"net":uuid}},
        {"type":"object","additionalProperties":false,"required":["kind","anchor"],"properties":{"kind":{"const":"derived"},"anchor":super::anchor_schema()}}
    ]})
}

/// Explicitly empty stored lists are not a second encoding of the absent assignment vocabulary.
pub(super) fn stored_bindings<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Vec<ClassBinding>, D::Error> {
    let bindings = Vec::<ClassBinding>::deserialize(de)?;
    if bindings.is_empty() || bindings.len() > MAX_BINDINGS {
        return Err(serde::de::Error::custom("Class bindings must be omitted or nonempty and bounded."));
    }
    Ok(bindings)
}

/// An active binding resolution remains typed for future routing/query/DRC consumers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoundResolution {
    pub address: NetAddress,
    pub bindings: Vec<ClassBinding>,
    pub resolution: net_classes::Resolution,
}

impl Registry {
    /// New assignment vocabulary selects a versioned project/instance envelope only when present.
    pub(crate) fn has_class_bindings(&self) -> bool {
        self.records.iter().any(|record| !record.class_bindings.is_empty())
    }
}
impl Project {
    /// Resolve a scoped target without allocating identities. Historical targets may be inspected
    /// but must not be assigned or consumed as current dimensional authority.
    fn class_target(&self, target: &ClassTarget) -> Result<(Option<Uuid>, Option<usize>), CommandError> {
        match target {
            ClassTarget::Persistent { project, scope, net } => {
                if *project != self.id || *scope != self.net_scope() || self.net_registry.record(*net).is_none() {
                    return Err(CommandError::InvalidParams("Unknown or foreign scoped net identity.".into()));
                }
                Ok((Some(*net), None))
            }
            ClassTarget::Derived { anchor } => {
                if anchor.project != self.id || anchor.scope != self.net_scope() {
                    return Err(CommandError::InvalidParams("Foreign derived net owner.".into()));
                }
                let graph = Graph::build(self).map_err(CommandError::InvalidParams)?;
                let index = graph
                    .network(anchor)
                    .filter(|index| graph.networks[*index].meaningful())
                    .ok_or_else(|| CommandError::InvalidParams("Class assignment requires a current meaningful net.".into()))?;
                let id = self
                    .net_registry
                    .records
                    .iter()
                    .find(|record| matches!(record.state, State::Active {}) && graph.network(&record.primary) == Some(index))
                    .map(|record| record.id);
                Ok((id, Some(index)))
            }
        }
    }

    /// Resolve active saved assignments against their exact owner's defaults, preserving binding
    /// origins and equal-precedence conflicts. Ambiguous/retired identities are never a live result.
    pub fn resolve_net_assignment(&self, address: NetAddress) -> Result<BoundResolution, CommandError> {
        self.class_target(&ClassTarget::Persistent { project: address.project, scope: address.scope, net: address.net })?;
        let record = self.net_registry.record(address.net).ok_or_else(|| CommandError::InvalidParams("Unknown net identity.".into()))?;
        if !matches!(record.state, State::Active {}) {
            return Err(CommandError::Unavailable("This net identity is retired or awaiting an explicit identity resolution.".into()));
        }
        let classes: Vec<_> = record.class_bindings.iter().map(|binding| binding.class).collect::<BTreeSet<_>>().into_iter().collect();
        Ok(BoundResolution { address, bindings: record.class_bindings.clone(), resolution: self.resolve_net_classes(&classes, self.board_target())? })
    }

    /// Inspect assignments/conflicts without editing documents or promoting derived networks.
    pub fn inspect_net_classes(&self, target: &ClassTarget) -> Result<Value, CommandError> {
        let (id, _) = self.class_target(target)?;
        if let Some(id) = id {
            let record = self.net_registry.record(id).ok_or_else(|| CommandError::InvalidParams("Unknown net identity.".into()))?;
            let resolution = if matches!(record.state, State::Active {}) {
                Some(self.resolve_net_assignment(NetAddress { project: self.id, scope: self.net_scope(), net: id })?.resolution)
            } else {
                None
            };
            let status = match (&record.state, &resolution) {
                (State::Retired { .. }, _) => "retired",
                (State::Ambiguous { .. }, _) => "identity_ambiguous",
                (_, Some(resolution)) if !resolution.conflicts.is_empty() => "conflicted",
                _ => "ready",
            };
            Ok(
                json!({"target":ClassTarget::Persistent{project:self.id,scope:self.net_scope(),net:id},"bindings":record.class_bindings,"status":status,"resolution":resolution}),
            )
        } else {
            Ok(json!({"target":target,"bindings":[],"status":"derived","resolution":self.resolve_net_classes(&[],self.board_target())?}))
        }
    }

    /// Until class-aware consumers are delivered, required dimensional bindings cannot be
    /// silently evaluated using only board defaults. Empty classification sets add no dimensions.
    pub(crate) fn require_supported_class_consumer(&self) -> Result<(), CommandError> {
        if self
            .net_registry
            .records
            .iter()
            .filter(|record| matches!(record.state, State::Active {}))
            .flat_map(|record| &record.class_bindings)
            .any(|binding| self.net_classes.get(binding.class).is_some_and(|class| !class.rules.is_empty()))
        {
            Err(CommandError::Unavailable("This operation does not yet support assigned net-class rules.".into()))
        } else {
            Ok(())
        }
    }
}

impl Session {
    /// Replace one complete class set atomically. New meaningful targets are promoted in the same
    /// history step; empty derived assignments/no-ops never allocate or rewrite identity records.
    pub(crate) fn set_net_classes(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let target = ClassTarget::parse(&params["target"])?;
        let classes = net_classes::parse_set(&params["classes"])?;
        for id in &classes {
            if original.net_classes.get(*id).is_none() {
                return Err(CommandError::InvalidParams("Unknown net class.".into()));
            }
        }
        let (existing, index) = original.class_target(&target)?;
        if let Some(id) = existing {
            let record = original.net_registry.record(id).ok_or_else(|| CommandError::InvalidParams("Unknown net identity.".into()))?;
            if !matches!(record.state, State::Active {}) {
                return Err(CommandError::InvalidParams("Class assignment requires an active net identity.".into()));
            }
            if record.class_bindings.iter().map(|binding| binding.class).collect::<BTreeSet<_>>() == classes.iter().copied().collect() {
                return Ok(());
            }
        } else if classes.is_empty() {
            return Ok(());
        }
        let mut candidate =
            if original.layout.is_none() { original.with_board_folders(".").map_err(CommandError::InvalidParams)? } else { original.clone() };
        if !candidate.has_sheet_occurrences() {
            candidate = candidate.with_sheet_occurrences().map_err(CommandError::InvalidParams)?;
        }
        let mut registry = (*candidate.net_registry).clone();
        let id = if let Some(id) = existing {
            id
        } else {
            let ClassTarget::Derived { anchor } = target else {
                return Err(CommandError::InvalidParams("A derived net needs a current anchor.".into()));
            };
            let graph = Graph::build(original).map_err(CommandError::InvalidParams)?;
            let mut ids = registry.records.iter().map(|record| record.id).collect();
            let id = allocate(&mut ids)?;
            registry.records.push(
                graph.networks[index.ok_or_else(|| CommandError::InvalidParams("Missing derived network.".into()))?]
                    .record(id, Some(&anchor))
                    .map_err(CommandError::InvalidParams)?,
            );
            id
        };
        let record = registry
            .records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| CommandError::InvalidParams("Missing assignment identity.".into()))?;
        let wanted: BTreeSet<_> = classes.into_iter().collect();
        record.class_bindings.retain(|binding| wanted.contains(&binding.class));
        for class in wanted {
            if !record.class_bindings.iter().any(|binding| binding.class == class) {
                record.class_bindings.push(ClassBinding { class, origin: id });
            }
        }
        record.class_bindings.sort();
        registry.records.sort_by_key(|record| record.id);
        candidate.net_registry = Arc::new(registry);
        candidate.net_registry.validate(&candidate).map_err(CommandError::InvalidParams)?;
        self.change(candidate)
    }
    /// Queries return binding status even when full project inspection is suppressed by MCP.
    pub(crate) fn inspect_net_class_target(&self, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        let assignments = if let Some(target) = params.get("target") {
            project.inspect_net_classes(&ClassTarget::parse(target)?)?
        } else {
            project.inspect_net_registry().map_err(CommandError::InvalidParams)?
        };
        Ok(json!({"revision":self.revision(),"net_assignments":assignments}))
    }
}
