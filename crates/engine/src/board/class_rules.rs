//! Snapshot-derived class policy for addressed copper. Membership remains in the existing graph.
#[cfg(test)]
mod tests;
use super::{clearance::BoardGeometry, rules, view::BoardView};
use crate::{
    CommandError, Project,
    net_classes::{self, Property},
    nets::{ClassBinding, NetAddress},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Inspect defaults, hard requirements and conflicts at one current copper contact.
pub const ROUTE_RULES: &str = "pcb.route_rules";

/// Effective dimensions retain the scoped assigning nets, original binding sources and exact
/// class revisions. Conflicts remain inspectable independently of the consuming operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub nets: Vec<NetAddress>,
    pub bindings: Vec<ClassBinding>,
    pub resolution: net_classes::Resolution,
}

impl Applied {
    /// Effective spacing for the addressed net/object; unrelated preference conflicts do not
    /// make this independent hard minimum unavailable.
    pub fn clearance_nm(&self) -> Result<i64, CommandError> {
        self.value(Property::ClearanceNm)
    }
    /// Empty-rule classifications add identity/provenance but no dimensional requirement.
    pub fn has_dimensions(&self) -> bool {
        self.resolution.contributions.values().flatten().any(|contribution| matches!(contribution.source, net_classes::Source::Class { .. }))
    }
    /// Existing copper checks consume hard dimensions without choosing or validating an unused
    /// routing preference. The resolver always supplies hard values, even for contradictory bounds.
    pub(crate) fn hard_values(&self, baseline: rules::Values) -> rules::Values {
        use Property::*;
        let value = |property, fallback| self.value(property).unwrap_or(fallback);
        rules::Values {
            clearance_nm: value(ClearanceNm, baseline.clearance_nm),
            min_track_width_nm: value(MinTrackWidthNm, baseline.min_track_width_nm),
            edge_clearance_nm: value(EdgeClearanceNm, baseline.edge_clearance_nm),
            min_annular_ring_nm: value(MinAnnularRingNm, baseline.min_annular_ring_nm),
            min_via_diameter_nm: value(MinViaDiameterNm, baseline.min_via_diameter_nm),
            max_via_diameter_nm: value(MaxViaDiameterNm, baseline.max_via_diameter_nm),
            min_via_drill_nm: value(MinViaDrillNm, baseline.min_via_drill_nm),
            max_via_drill_nm: value(MaxViaDrillNm, baseline.max_via_drill_nm),
            ..baseline
        }
    }
    /// Read one resolved dimension, refusing an unresolved preference rather than inventing one.
    pub(crate) fn value(&self, property: Property) -> Result<i64, CommandError> {
        self.resolution
            .values
            .get(&property)
            .copied()
            .flatten()
            .ok_or_else(|| CommandError::Unavailable(format!("Net-class conflict leaves {} unresolved.", property.as_str())))
    }

    /// Hard-bound contradictions affect only operations needing those bounds. A conflicting
    /// preference does not prevent checking existing copper or explicitly choosing a valid size.
    pub(crate) fn require(&self, properties: &[Property]) -> Result<(), CommandError> {
        if let Some(conflict) = self.resolution.conflicts.iter().find(|conflict| {
            conflict.properties.iter().all(|property| !property.is_preference())
                && conflict.properties.iter().any(|property| properties.contains(property))
        }) {
            return Err(CommandError::Unavailable(format!("Net-class rules conflict: {}", conflict.message)));
        }
        Ok(())
    }

    /// Resolve the upcoming width only when omitted, and enforce its applicable hard minimum.
    pub fn track_width(&self, explicit: Option<i64>) -> Result<i64, CommandError> {
        self.require(&[Property::MinTrackWidthNm])?;
        let width = match explicit {
            Some(width) => width,
            None => self.value(Property::PreferredTrackWidthNm)?,
        };
        if width < self.value(Property::MinTrackWidthNm)?.max(super::track::MIN_WIDTH_NM) || width > super::track::MAX_WIDTH_NM {
            return Err(CommandError::InvalidParams("Track width cannot weaken an applicable net-class minimum.".into()));
        }
        Ok(width)
    }

    /// Defaults and hard limits stay distinct for each via operand. Explicit valid dimensions
    /// can resolve a preference choice, but cannot bypass diameter, drill or annular-ring bounds.
    pub fn via_sizes(&self, diameter: Option<i64>, drill: Option<i64>) -> Result<(i64, i64), CommandError> {
        use Property::*;
        self.require(&[MinViaDiameterNm, MaxViaDiameterNm, MinViaDrillNm, MaxViaDrillNm, MinAnnularRingNm])?;
        let diameter = match diameter {
            Some(value) => value,
            None => self.value(PreferredViaDiameterNm)?,
        };
        let drill = match drill {
            Some(value) => value,
            None => self.value(PreferredViaDrillNm)?,
        };
        if diameter < self.value(MinViaDiameterNm)?
            || diameter > self.value(MaxViaDiameterNm)?
            || drill < self.value(MinViaDrillNm)?
            || drill > self.value(MaxViaDrillNm)?
            || diameter <= drill
            || diameter - drill < 2 * self.value(MinAnnularRingNm)?
        {
            return Err(CommandError::InvalidParams("Via dimensions cannot weaken applicable net-class limits.".into()));
        }
        Ok((diameter, drill))
    }
}

/// Deduplicated object policies explain each finding/query without copying complete rule
/// contributions into every geometric pair. Indices are local to this captured result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Context {
    pub policies: Vec<Applied>,
    pub objects: BTreeMap<String, usize>,
    #[serde(skip)]
    pub(crate) by_object: Vec<Option<usize>>,
}

impl Context {
    /// Inspect one exact copper address from this captured board, without name-based net lookup.
    pub fn rules_for(&self, object: &str) -> Option<&Applied> {
        self.objects.get(object).and_then(|index| self.policies.get(*index))
    }
    /// Unclassified objects retain board defaults; classified objects use the captured hard limits.
    pub(crate) fn limits_for(&self, object: &str, baseline: rules::Values) -> rules::Values {
        self.rules_for(object).map_or(baseline, |policy| policy.hard_values(baseline))
    }
    /// Resolve an existing geometry object's policy under this exact snapshot projection.
    pub(crate) fn object(&self, index: usize) -> Option<&Applied> {
        self.by_object.get(index).copied().flatten().and_then(|policy| self.policies.get(policy))
    }

    /// Broad-phase expansion covers every captured class requirement; narrow-phase decisions
    /// still use the independent requirements of their actual pair.
    pub(crate) fn max_clearance(&self, baseline: i64) -> i64 {
        self.policies.iter().filter_map(|policy| policy.value(Property::ClearanceNm).ok()).fold(baseline, net_classes::pair_clearance)
    }

    /// The same symmetric spacing rule as #300, without resolving unrelated route preferences.
    pub(crate) fn clearance(&self, left: Option<usize>, right: usize, baseline: i64) -> i64 {
        let value = |index| self.object(index).and_then(|policy| policy.value(Property::ClearanceNm).ok()).unwrap_or(baseline);
        net_classes::pair_clearance(left.map_or(baseline, value), value(right))
    }
}

impl BoardView<'_> {
    /// Resolve classes of copper actually touching a point on an addressed copper layer. No
    /// assignments means no geometry preparation. Unknown free space has board defaults only;
    /// through-via queries omit the layer and combine contacts across all copper layers.
    pub fn route_rules_at(self, at: crate::schematic::Point, layer: Option<Uuid>) -> Result<Option<Applied>, CommandError> {
        if !crate::wire::valid_point(&at) {
            return Err(CommandError::InvalidParams("Route contact is out of range.".into()));
        }
        if let Some(layer) = layer
            && !self.layers().iter().any(|record| record.id == layer && record.purpose == super::setup::LayerPurpose::Copper)
        {
            return Err(CommandError::InvalidParams("Route rules require a copper layer of the addressed board.".into()));
        }
        let Some(rules) = NetRules::new(self, self.effective_rules())? else { return Ok(None) };
        let geometry = BoardGeometry::new(self);
        let contact = super::geometry::Contour::capsule(at, at, 0.);
        let members: Vec<_> = geometry
            .items
            .iter()
            .filter(|item| layer.is_none_or(|layer| item.layers.contains(&layer)) && item.contour.gap(&contact) <= 0.)
            .flat_map(|item| item.intended_nets.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        rules.resolve(self.project(), &members)
    }
}

impl crate::Session {
    /// Read a scoped, current routing contact without changing revision, history or saved bytes.
    /// Query results remain available when MCP suppresses ordinary project inspection.
    pub(crate) fn route_rule_query(&self, params: &serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(serde_json::Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
        let at = serde_json::from_value(params["at"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        let layer = params
            .get("layer")
            .map(|layer| serde_json::from_value(layer.clone()))
            .transpose()
            .map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        let applied = view.route_rules_at(at, layer)?;
        Ok(
            serde_json::json!({"revision":self.revision(),"route_rules":{"board":target,"at":at,"layer":layer,"baseline":view.effective_rules(),"applied":applied}}),
        )
    }
}

/// Current stable assignments projected onto this snapshot's accepted physical memberships.
/// The map is transient, qualified by its owner's snapshot, and never saved as a second netlist.
pub(crate) struct NetRules {
    baseline: rules::Effective,
    by_membership: BTreeMap<usize, BTreeSet<Uuid>>,
    clearances: BTreeMap<usize, i64>,
}

impl NetRules {
    /// Unclassified snapshots skip graph/policy construction. Accepted physical membership
    /// remains authoritative while pending schematic changes retain their current class intent.
    pub(crate) fn new(view: BoardView<'_>, baseline: rules::Effective) -> Result<Option<Self>, CommandError> {
        let project = view.project();
        let terminals = project.classified_terminals()?;
        if terminals.is_empty() {
            return Ok(None);
        }
        let accepted = project.board.baseline.as_ref().map(|baseline| baseline.terminal_net_index());
        let graph = accepted.is_none().then(|| project.electrical_connectivity());
        let mut by_membership = BTreeMap::<usize, BTreeSet<Uuid>>::new();
        for ((component, terminal), id) in terminals {
            let membership = if let Some(accepted) = &accepted {
                accepted.get(&(component, terminal.as_str())).copied()
            } else {
                project.component_terminal(component, &terminal).and_then(|terminal| graph.as_ref()?.net_index_for(terminal))
            };
            if let Some(membership) = membership {
                by_membership.entry(membership).or_default().insert(id);
            }
        }
        if by_membership.is_empty() {
            return Ok(None);
        }
        let mut result = Self { baseline, by_membership, clearances: BTreeMap::new() };
        for (&member, ids) in &result.by_membership {
            let ids: Vec<_> = ids.iter().copied().collect();
            let policy = result.resolve_ids(project, &ids)?.ok_or_else(|| CommandError::InvalidParams("Missing net rule policy.".into()))?;
            result.clearances.insert(member, policy.value(Property::ClearanceNm)?);
        }
        Ok(Some(result))
    }

    /// Resolve only spacing for probe/overlay memberships without recomputing all dimensions.
    pub(crate) fn clearance(&self, members: &[usize]) -> i64 {
        members.iter().filter_map(|member| self.clearances.get(member).copied()).fold(self.baseline.values.clearance_nm, net_classes::pair_clearance)
    }

    /// Class sources follow stable terminal membership, including unions on conflicting copper
    /// islands. Unknown copper has no invented class or persistent net identity.
    fn ids(&self, members: &[usize]) -> Vec<Uuid> {
        members.iter().flat_map(|member| self.by_membership.get(member).into_iter().flatten().copied()).collect::<BTreeSet<_>>().into_iter().collect()
    }

    /// Combine only the requested memberships with the shared dimensional resolver and retain
    /// every original assignment. Names and physical traversal order never select a winner.
    pub(crate) fn resolve(&self, project: &Project, members: &[usize]) -> Result<Option<Applied>, CommandError> {
        self.resolve_ids(project, &self.ids(members))
    }

    /// Resolve canonical saved identities once per distinct geometry policy during capture.
    fn resolve_ids(&self, project: &Project, ids: &[Uuid]) -> Result<Option<Applied>, CommandError> {
        if ids.is_empty() {
            return Ok(None);
        }
        let mut bindings = BTreeSet::new();
        for id in ids {
            let record = project.net_registry.record(*id).ok_or_else(|| CommandError::InvalidParams("Missing classified identity.".into()))?;
            bindings.extend(record.class_bindings.iter().cloned());
        }
        let bindings: Vec<_> = bindings.into_iter().collect();
        let classes: Vec<_> = bindings.iter().map(|binding| binding.class).collect::<BTreeSet<_>>().into_iter().collect();
        let resolution = project.resolve_net_classes_with_rules(&classes, project.board_target(), Some(&self.baseline))?;
        Ok(Some(Applied {
            nets: ids.iter().map(|net| NetAddress { project: project.id, scope: project.net_scope(), net: *net }).collect(),
            bindings,
            resolution,
        }))
    }

    /// Snapshot object indices refer to deduplicated policies in the result, not saved net IDs.
    pub(super) fn capture(&self, view: BoardView<'_>, geometry: &BoardGeometry) -> Result<Context, CommandError> {
        self.capture_named(view.project(), geometry.items.iter().map(|item| (item.name.clone(), item)))
    }

    /// Overlay identities remain separate from source geometry while sharing the same canonical
    /// policy deduplication. No overlay membership is installed in the project or base index.
    pub(crate) fn capture_named<'a>(
        &self,
        project: &Project,
        items: impl IntoIterator<Item = (String, &'a super::clearance::Object)>,
    ) -> Result<Context, CommandError> {
        let mut context = Context::default();
        let mut unique = BTreeMap::new();
        for (name, item) in items {
            let ids = self.ids(&item.intended_nets);
            let index = if ids.is_empty() {
                None
            } else if let Some(index) = unique.get(&ids) {
                Some(*index)
            } else {
                let policy = self.resolve_ids(project, &ids)?.ok_or_else(|| CommandError::InvalidParams("Missing object rule policy.".into()))?;
                let index = context.policies.len();
                context.policies.push(policy);
                unique.insert(ids, index);
                Some(index)
            };
            context.by_object.push(index);
            if let Some(index) = index {
                context.objects.insert(name, index);
            }
        }
        Ok(context)
    }
}
