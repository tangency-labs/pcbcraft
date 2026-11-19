//! Select final relation-relevant islands by independent physical and electrical deltas.

use super::transaction::reason;
use crate::{
    CommandError, Project,
    board::{
        self,
        geometry::Contour,
        track::{self, Track, Via},
    },
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Land(String, String),
    Track(Uuid),
    Via(Uuid),
}

#[derive(PartialEq, Eq)]
enum Binding {
    Track,
    Land { pin: Option<(u64, String)>, assigned: bool },
    Via { state: &'static str, target: Option<crate::nets::NetAddress> },
}

struct Node<'a> {
    key: Key,
    contour: Contour,
    layers: Vec<Uuid>,
    track: Option<&'a Track>,
    via: Option<&'a Via>,
    binding: Binding,
    net: Option<usize>,
}

impl Node<'_> {
    /// Compare only modeled physical geometry and native copper records, not inferred net IDs.
    fn same_copper(&self, after: &Self) -> bool {
        self.contour.core == after.contour.core
            && self.contour.radius == after.contour.radius
            && self.layers == after.layers
            && self.track == after.track
            && self.via == after.via
    }

    /// Test original direct contact with native layer occupancy and contour policy.
    fn touches(&self, other: &Self) -> bool {
        self.layers.iter().any(|layer| other.layers.contains(layer))
            && match (&self.key, &other.key) {
                (Key::Land(..), Key::Land(..)) => self.contour.overlaps(&other.contour),
                _ => self.contour.touches(&other.contour),
            }
    }
}

struct Copper<'a> {
    lands: Vec<track::Land>,
    tracks: &'a [Track],
    vias: &'a [Via],
    nodes: Vec<Node<'a>>,
    index: BTreeMap<Key, usize>,
    memberships: board::via_nets::Memberships,
}

impl<'a> Copper<'a> {
    /// Project stable native node keys and snapshot-local intrinsic membership only once.
    fn read(project: &'a Project) -> Option<Self> {
        let view = project.board_view(project.board_target()?)?;
        let lands = track::lands(view);
        let (tracks, vias) = (view.tracks(), view.vias());
        let memberships = project.board_intent_memberships();
        let net = board::via_nets::node_membership(project, &lands, tracks.len(), vias, &memberships);
        let layers = project
            .board
            .setup
            .iter()
            .flat_map(|s| s.layers())
            .filter(|layer| layer.purpose == board::setup::LayerPurpose::Copper)
            .map(|layer| layer.id)
            .collect::<Vec<_>>();
        let mut nodes = Vec::with_capacity(net.len());
        for land in &lands {
            let pin =
                board::Pad::parse(&land.key.0, &land.key.1).and_then(|pad| project.board.component_pin_for(pad)).map(|(id, p)| (id, p.to_owned()));
            let membership = net[nodes.len()];
            nodes.push(Node {
                key: Key::Land(land.key.0.clone(), land.key.1.clone()),
                contour: land.contour(),
                layers: land.layers.clone(),
                track: None,
                via: None,
                net: membership,
                binding: Binding::Land { pin, assigned: membership.is_some() },
            });
        }
        for track in tracks {
            nodes.push(Node {
                key: Key::Track(track.id),
                contour: Contour::capsule(track.start, track.end, track.width_nm as f64 / 2.),
                layers: vec![track.layer],
                track: Some(track),
                via: None,
                net: None,
                binding: Binding::Track,
            });
        }
        for via in vias {
            let state = via.net_assignment.map_or("automatic", |a| project.via_net_state(a));
            let membership = net[nodes.len()];
            nodes.push(Node {
                key: Key::Via(via.id),
                contour: Contour::capsule(via.at, via.at, via.diameter_nm as f64 / 2.),
                layers: layers.clone(),
                track: None,
                via: Some(via),
                net: membership,
                binding: Binding::Via { state, target: (state == "active").then_some(via.net_assignment).flatten() },
            });
        }
        let index = nodes.iter().enumerate().map(|(i, node)| (node.key.clone(), i)).collect();
        Some(Self { lands, tracks, vias, nodes, index, memberships })
    }

    /// Compare the partition on the same local nodes, not net names, ordinal changes or remote peers.
    fn electrical_changed(&self, before: &Self, group: &[usize]) -> bool {
        let mut old_to_new = BTreeMap::new();
        let mut new_to_old = BTreeMap::new();
        group.iter().any(|&i| {
            let after = &self.nodes[i];
            let Some(old) = before.index.get(&after.key).map(|&j| &before.nodes[j]) else { return true };
            old.binding != after.binding
                || match (old.net, after.net) {
                    (None, None) => false,
                    (Some(a), Some(b)) => old_to_new.insert(a, b).is_some_and(|v| v != b) || new_to_old.insert(b, a).is_some_and(|v| v != a),
                    _ => true,
                }
        })
    }
}

/// Refuse a short only on a final changed island containing the land of a relation-named body.
/// This is independent of native edit checks, uses native contacts, and never attaches copper.
pub(super) fn check(previous: &Project, candidate: &Project, bodies: &BTreeSet<String>) -> Result<(), CommandError> {
    if bodies.is_empty() {
        return Ok(());
    }
    let Some(after) = Copper::read(candidate) else { return Ok(()) };
    let (copper, groups) = track::islands(&after.lands, after.tracks, after.vias);
    let groups = track::with_pad_contacts(&after.lands, &copper, groups);
    let Some(before) = Copper::read(previous) else { return Ok(()) };
    let vanished: Vec<_> = before.nodes.iter().filter(|node| !after.index.contains_key(&node.key)).collect();
    for group in &groups {
        if !group.iter().any(|&i| matches!(&after.nodes[i].key, Key::Land(fp, _) if bodies.contains(fp))) {
            continue;
        }
        // Disappeared-node triggers use original direct contacts with surviving nodes.
        let physical = group.iter().any(|&i| {
            let node = &after.nodes[i];
            let Some(&j) = before.index.get(&node.key) else { return true };
            let old = &before.nodes[j];
            !old.same_copper(node) || vanished.iter().any(|removed| removed.touches(old))
        });
        let electrical = !physical && after.electrical_changed(&before, group);
        if !physical && !electrical {
            continue;
        }
        #[cfg(test)]
        super::probes::hit(super::probes::Op::Marked { nodes: group.iter().map(|&i| after.nodes[i].key.clone()).collect(), physical, electrical });
        if let Some((a, b)) = track::intrinsic_short(group, &after.lands, &after.memberships, |i| after.nodes[i].net) {
            return Err(reason(
                "affected_short",
                &format!("This edit would short {a} and {b} in an affected relation island."),
                json!({"nets":[a,b]}),
            ));
        }
    }
    Ok(())
}
