//! Authored via intent and transient projection onto accepted physical net memberships.
use super::{
    track::{Land, Via},
    view::BoardView,
};
use crate::{
    CommandError, Project,
    nets::{ClassTarget, NetAddress, State},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Assign selected vias to one current scoped net, or return them to automatic mode.
pub const SET_VIA_NET: &str = "pcb.set_via_net";

/// Snapshot-local memberships include synthetic indices for active intent with no accepted pads.
/// These indices are never persisted and never join physically separate islands.
#[derive(Default)]
pub(crate) struct Memberships {
    pub ids: BTreeMap<Uuid, usize>,
    pub names: BTreeMap<usize, String>,
}

impl Project {
    /// Historical or foreign saved targets remain inspectable; only active local targets grant intent.
    pub fn via_net_state(&self, address: NetAddress) -> &'static str {
        if address.project != self.id || address.scope != self.net_scope() {
            return "foreign";
        }
        match self.net_registry.record(address.net).map(|r| &r.state) {
            Some(State::Active {}) => "active",
            Some(State::Ambiguous { .. }) => "ambiguous",
            Some(State::Retired { .. }) => "retired",
            None => "missing",
        }
    }

    /// Promote a current chooser target inside its eventual copper transaction; inactive targets fail.
    pub(crate) fn with_via_target(&self, value: &Value) -> Result<(Self, NetAddress), CommandError> {
        let (next, net) = self.with_promoted_net(&ClassTarget::parse(value)?)?;
        let address = NetAddress { project: next.id, scope: next.net_scope(), net };
        if next.via_net_state(address) != "active" {
            return Err(CommandError::InvalidParams("Choose an active net; unresolved identities require review.".into()));
        }
        Ok((next, address))
    }
}

/// Explicit native addresses must be present objects with non-nil identities; absence is automatic.
pub(crate) fn stored_assignment<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<NetAddress>, D::Error> {
    use serde::Deserialize;
    let address = NetAddress::deserialize(de)?;
    if address.project.is_nil() || address.scope.id.is_nil() || address.net.is_nil() {
        return Err(serde::de::Error::custom("Via net addresses require non-nil identities."));
    }
    Ok(Some(address))
}

impl BoardView<'_> {
    /// Inspect saved intent against the exact board owner without exposing mutable project access.
    pub fn via_net_state(self, address: NetAddress) -> &'static str {
        self.project().via_net_state(address)
    }

    /// Read the explicit chooser target's dimensional policy without promotion or geometry edits.
    pub fn via_target_rules(self, target: &ClassTarget) -> Result<Option<super::class_rules::Applied>, CommandError> {
        Ok(self.project().resolve_net_target(target)?.map(|r| super::class_rules::Applied {
            nets: vec![r.address],
            bindings: r.bindings,
            resolution: r.resolution,
        }))
    }

    /// Inspect authored intent independently of resolved island membership and physical pad contact.
    /// Unsupported circular geometry leaves derived fields null and reports its error, while
    /// retaining the authored assignment and its registry state.
    pub fn inspect_vias(self) -> Vec<Value> {
        if self.vias().is_empty() {
            return Vec::new();
        }
        let membership = self.copper_with_nets();
        let pad_islands: BTreeSet<_> = membership.as_ref().map(|(copper, _)| copper.pads.values().copied().collect()).unwrap_or_default();
        self.vias()
            .iter()
            .map(|via| {
                let mut value = via.inspect();
                value["assignment_mode"] = json!(if via.net_assignment.is_some() { "explicit" } else { "automatic" });
                value["net_state"] = json!(via.net_assignment.map(|a| self.project().via_net_state(a)).unwrap_or("automatic"));
                match &membership {
                    Ok((copper, nets)) => {
                        value["net"] = json!(nets.get(&via.id));
                        value["connected_to_pad"] = json!(copper.vias.get(&via.id).is_some_and(|island| pad_islands.contains(island)));
                    }
                    Err(error) => {
                        value["net"] = Value::Null;
                        value["connected_to_pad"] = Value::Null;
                        value["geometry_error"] = json!(error);
                    }
                }
                value
            })
            .collect()
    }
}

/// One node's intrinsic intent: pad membership or an active authored via target, never a name.
pub(super) fn node_membership(project: &Project, lands: &[Land], tracks: usize, vias: &[Via], memberships: &Memberships) -> Vec<Option<usize>> {
    node_membership_with_arcs(project, lands, tracks, vias, 0, memberships)
}

/// One node's intrinsic intent across pads, tracks, vias and circular arcs.
pub(super) fn node_membership_with_arcs(
    project: &Project,
    lands: &[Land],
    tracks: usize,
    vias: &[Via],
    arcs: usize,
    memberships: &Memberships,
) -> Vec<Option<usize>> {
    let accepted = project.board.baseline.as_ref().map(|b| b.terminal_net_index());
    let graph = accepted.is_none().then(|| project.electrical_connectivity());
    lands
        .iter()
        .map(|land| {
            let pad = super::Pad::parse(&land.key.0, &land.key.1)?;
            let (component, terminal) = project.board.component_pin_for(pad)?;
            if let Some(accepted) = &accepted {
                accepted.get(&(component, terminal)).copied()
            } else {
                graph.as_ref()?.net_index_for(project.component_terminal(component, terminal)?)
            }
        })
        .chain(std::iter::repeat_n(None, tracks))
        .chain(vias.iter().map(|via| {
            let address = via.net_assignment?;
            (project.via_net_state(address) == "active").then(|| memberships.ids.get(&address.net).copied()).flatten()
        }))
        .chain(std::iter::repeat_n(None, arcs))
        .collect()
}

#[cfg(test)]
mod tests;
