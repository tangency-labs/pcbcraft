//! Immutable board copper queries. Candidate enumeration is conservative and independent of
//! contact/separation predicates; the exhaustive path is a reference for later spatial indexes.
use super::{
    geometry::Contour,
    track::{Land, islands, lands, with_pad_contacts},
    view::{BoardTarget, BoardView},
};
use crate::{CommandError, Project, Session, schematic::Point};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

/// Runtime source authority. Snapshot-local object/net indices have meaning only under this
/// authority; neither these indices nor this token are persisted as net bindings.
#[derive(Clone, Debug)]
pub struct Authority {
    snapshot: Arc<Project>,
    session: Arc<()>,
    target: BoardTarget,
    opening: u64,
    revision: u64,
    clearance_nm: i64,
    rules: super::rules::Effective,
}

impl Authority {
    /// The explicit project and physical board checked.
    pub fn target(&self) -> BoardTarget {
        self.target
    }
    /// The session revision captured before preparing geometry and intended membership.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Effective clearance for this result, including a request-local rule override.
    pub fn clearance_nm(&self) -> i64 {
        self.clearance_nm
    }
    /// Shared resolved values and provenance captured with the immutable board.
    pub fn rules(&self) -> &super::rules::Effective {
        &self.rules
    }
    /// Reject another session, reopen, Undo/Redo, geometry/connectivity edit or replaced snapshot.
    /// Rule overrides are immutable inputs: a different override requires a new query.
    pub fn is_current(&self, session: &Session) -> bool {
        Arc::ptr_eq(&self.session, &session.clearance_identity)
            && self.opening == session.history_root_revision()
            && self.revision == session.revision()
            && session.snapshot().is_some_and(|current| Arc::ptr_eq(&self.snapshot, &current))
            && self.snapshot.board_view(self.target).is_some()
    }
}

/// An addressed copper object in this query's source snapshot. Membership indices are sorted,
/// snapshot-local intended nets, never display names. Empty membership stays unknown; a set
/// with several entries preserves a conflicting-net island rather than choosing its first net.
#[derive(Clone, Debug)]
pub struct Object {
    pub name: String,
    pub contour: Contour,
    pub layers: Vec<Uuid>,
    pub intended_nets: Vec<usize>,
    pub at: Point,
    pub(crate) island: usize,
    pub(crate) net: Option<String>,
}

/// Exact represented-contour decision after candidate enumeration. Contact is independent of
/// the strict clearance comparison, so conflicting-net tangency is reported even at zero rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// Source object index, or absent for a disposable probe.
    pub first: Option<usize>,
    pub second: usize,
    pub gap_nm: f64,
    pub touches: bool,
    pub clearance_violation: bool,
    pub conflicting_nets: bool,
}

/// Derived decisions retain their authority even when the caller drops the prepared query.
#[derive(Clone, Debug)]
pub struct Results {
    pub authority: Authority,
    pub decisions: Vec<Decision>,
}

/// A prepared query over one immutable board snapshot and one effective clearance. This simple
/// implementation retains a sweep for board pairs and a linear scan for disposable probes.
pub struct ClearanceQuery {
    authority: Authority,
    geometry: BoardGeometry,
}

impl Session {
    /// Capture a board and effective rule without changing revision, history or project bytes.
    /// Unknown targets, absent projects and out-of-range clearances fail before preparation.
    pub fn clearance_query(&self, target: BoardTarget, clearance_nm: i64) -> Result<ClearanceQuery, CommandError> {
        self.clearance_query_with_rules(target, &serde_json::json!({"clearance_nm":clearance_nm}))
    }

    /// Capture saved board defaults, or diagnostic overrides that cannot weaken authored limits.
    /// Rule changes invalidate the returned authority just like copper or connectivity changes.
    pub fn clearance_query_with_rules(&self, target: BoardTarget, overrides: &serde_json::Value) -> Result<ClearanceQuery, CommandError> {
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let view = snapshot.board_view(target).ok_or_else(|| CommandError::InvalidParams("unknown board target".into()))?;
        let rules = view.resolve_rules(overrides)?;
        let clearance_nm = rules.values.clearance_nm;
        let geometry = BoardGeometry::new(view);
        Ok(ClearanceQuery {
            authority: Authority {
                snapshot,
                session: self.clearance_identity.clone(),
                target,
                opening: self.history_root_revision(),
                revision: self.revision(),
                clearance_nm,
                rules,
            },
            geometry,
        })
    }
}

impl ClearanceQuery {
    /// Captured source, opening, board, revision and effective rule.
    pub fn authority(&self) -> &Authority {
        &self.authority
    }
    /// Stable pad/track/via order within this snapshot. Indices expire with this authority.
    pub fn objects(&self) -> &[Object] {
        &self.geometry.items
    }
    /// Evaluate conservative sweep candidates on shared layers. Same-island clearance stays
    /// suppressed, while contact between conflicting intended memberships remains observable.
    pub fn pairs(&self) -> Results {
        let mut decisions: Vec<_> = self.geometry.pairs(self.authority.clearance_nm).collect();
        decisions.sort_by_key(|d| (d.first, d.second));
        Results { authority: self.authority.clone(), decisions }
    }
    /// Exhaustively evaluate all shared-layer pairs, without spatial pruning. Later indexes can
    /// compare their violating/contact pairs against this deliberately simple reference.
    pub fn exhaustive_pairs(&self) -> Results {
        let mut decisions = Vec::new();
        for i in 0..self.geometry.items.len() {
            for j in i + 1..self.geometry.items.len() {
                if let Some(decision) = self.geometry.evaluate(i, j, self.authority.clearance_nm) {
                    decisions.push(decision);
                }
            }
        }
        Results { authority: self.authority.clone(), decisions }
    }
    /// Enumerate conservative obstacle indices for a contour on explicit copper layers.
    /// Equality is retained so tangency participates in zero-clearance contact checks.
    pub fn candidates(&self, contour: &Contour, layers: &[Uuid]) -> Result<Vec<usize>, CommandError> {
        let view = self.authority.snapshot.board_view(self.authority.target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
        if layers.is_empty() || layers.iter().any(|id| !view.layers().iter().any(|l| l.id == *id && l.purpose == super::setup::LayerPurpose::Copper))
        {
            return Err(CommandError::InvalidParams("explicit copper layers of the query board are required".into()));
        }
        let bounds = contour.bounds();
        Ok(self
            .geometry
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (share_layers(layers, &item.layers) && near_bounds(bounds, item.contour.bounds(), self.authority.clearance_nm as f64))
                    .then_some(index)
            })
            .collect())
    }
    /// Evaluate a disposable contour using an existing source object's intended membership, or
    /// unknown membership when absent. `intended_from` is an index from this query, never a saved
    /// net ID. The source object is replaced by the probe and excluded from obstacle results.
    /// Contacted memberships are combined before evaluation, so even an initially unknown
    /// probe bridging two intended nets reports a short at zero clearance. No physical island
    /// exemption applies to a moved/new probe; every contact is retested.
    pub fn probe(&self, contour: &Contour, layers: &[Uuid], intended_from: Option<usize>) -> Result<Results, CommandError> {
        let nets = match intended_from {
            Some(index) => {
                &self.geometry.items.get(index).ok_or_else(|| CommandError::InvalidParams("unknown source copper object".into()))?.intended_nets[..]
            }
            None => &[],
        };
        let candidates = self.candidates(contour, layers)?;
        let gaps: Vec<_> = candidates
            .into_iter()
            .filter(|index| Some(*index) != intended_from)
            .map(|index| (index, contour.gap(&self.geometry.items[index].contour)))
            .collect();
        let connected: Vec<_> = nets
            .iter()
            .copied()
            .chain(gaps.iter().filter(|(_, gap)| *gap <= 0.).flat_map(|(index, _)| self.geometry.items[*index].intended_nets.iter().copied()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let decisions = gaps
            .into_iter()
            .map(|(index, gap)| decision(None, index, gap, &connected, &self.geometry.items[index].intended_nets, false, self.authority.clearance_nm))
            .collect();
        Ok(Results { authority: self.authority.clone(), decisions })
    }
}

/// Prepared snapshot geometry shared by the public captured query and DRC's immutable BoardView.
pub(super) struct BoardGeometry {
    pub(crate) items: Vec<Object>,
    pub(super) lands: Vec<Land>,
    pub(crate) members: Vec<Vec<usize>>,
}

impl BoardGeometry {
    /// Resolve supported contours and the complete intended membership of every routed island.
    /// Keep legacy clearance islands separate from direct-pad contact traversal used for shorts.
    pub(crate) fn new(view: BoardView<'_>) -> Self {
        let lands = lands(view);
        let (copper, members) = islands(&lands, view.tracks(), view.vias());
        Self::from_islands(view, lands, copper, members)
    }

    /// Complete contour/membership projection from already evaluated legacy copper islands.
    pub(super) fn from_islands(view: BoardView<'_>, lands: Vec<Land>, copper: super::track::Copper, members: Vec<Vec<usize>>) -> Self {
        let merged = with_pad_contacts(&lands, &copper, members.clone());
        Self::from_contacts(view, lands, copper, members, merged)
    }

    /// Project memberships after a resumable caller has completed direct-pad contacts.
    pub(super) fn from_contacts(
        view: BoardView<'_>,
        lands: Vec<Land>,
        copper: super::track::Copper,
        members: Vec<Vec<usize>>,
        merged: Vec<Vec<usize>>,
    ) -> Self {
        let project = view.project();
        let baseline = project.board.baseline.as_ref();
        let accepted = baseline.map(|b| b.terminal_net_index());
        let graph = baseline.is_none().then(|| project.electrical_connectivity());
        let pad_nets: Vec<Option<usize>> = lands
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
            .collect();
        let names: Vec<Option<String>> = members.iter().map(|nodes| nodes.iter().find_map(|node| lands.get(*node)?.net.clone())).collect();
        let members = merged;
        let mut node_nets = vec![Vec::new(); lands.len() + view.tracks().len() + view.vias().len()];
        for nodes in &members {
            let nets: Vec<_> = nodes.iter().filter_map(|node| pad_nets.get(*node).copied().flatten()).collect::<BTreeSet<_>>().into_iter().collect();
            for &node in nodes {
                node_nets[node] = nets.clone();
            }
        }
        let mut items = Vec::new();
        for (index, land) in lands.iter().enumerate() {
            let island = copper.pads[&land.key];
            let at = Point { x_nm: land.corners.iter().map(|p| p.x_nm).sum::<i64>() / 4, y_nm: land.corners.iter().map(|p| p.y_nm).sum::<i64>() / 4 };
            items.push(Object {
                name: format!("{}:{}", land.key.0, land.key.1),
                contour: land.contour(),
                layers: land.layers.clone(),
                island,
                intended_nets: pad_nets[index].map(|net| vec![net]).unwrap_or_else(|| node_nets[index].clone()),
                net: land.net.clone().or_else(|| names[island].clone()),
                at,
            });
        }
        for (index, track) in view.tracks().iter().enumerate() {
            let island = copper.tracks[&track.id];
            items.push(Object {
                name: track.id.to_string(),
                contour: Contour::capsule(track.start, track.end, track.width_nm as f64 / 2.),
                layers: vec![track.layer],
                island,
                intended_nets: node_nets[lands.len() + index].clone(),
                net: names[island].clone(),
                at: Point { x_nm: track.start.x_nm / 2 + track.end.x_nm / 2, y_nm: track.start.y_nm / 2 + track.end.y_nm / 2 },
            });
        }
        let layers: Vec<_> = view.layers().into_iter().filter(|l| l.purpose == super::setup::LayerPurpose::Copper).map(|l| l.id).collect();
        for (index, via) in view.vias().iter().enumerate() {
            let island = copper.vias[&via.id];
            items.push(Object {
                name: via.id.to_string(),
                contour: Contour::capsule(via.at, via.at, via.diameter_nm as f64 / 2.),
                layers: layers.clone(),
                island,
                intended_nets: node_nets[lands.len() + view.tracks().len() + index].clone(),
                net: names[island].clone(),
                at: via.at,
            });
        }
        Self { items, lands, members }
    }
    /// Stream conservative shared-layer bound candidates independently of narrow-phase tests.
    /// Equality stays in the candidate set; neither names nor physical connectivity prune it.
    fn candidate_pairs(&self, clearance_nm: i64) -> impl Iterator<Item = (usize, usize)> + '_ {
        let clearance = clearance_nm as f64;
        let bounds: Vec<_> = self.items.iter().map(|item| item.contour.bounds()).collect();
        let mut order: Vec<_> = (0..self.items.len()).collect();
        order.sort_by(|a, b| bounds[*a][0].total_cmp(&bounds[*b][0]));
        let (mut position, mut next) = (0, 1);
        std::iter::from_fn(move || {
            while position < order.len() {
                if next >= order.len() {
                    position += 1;
                    next = position + 1;
                    continue;
                }
                let (i, j) = (order[position], order[next]);
                next += 1;
                if bounds[j][0] > bounds[i][2] + clearance {
                    position += 1;
                    next = position + 1;
                    continue;
                }
                if near_bounds(bounds[i], bounds[j], clearance) && share_layers(&self.items[i].layers, &self.items[j].layers) {
                    return Some((i.min(j), i.max(j)));
                }
            }
            None
        })
    }
    /// Evaluate candidates without retaining an all-pairs intermediate table.
    fn pairs(&self, clearance_nm: i64) -> impl Iterator<Item = Decision> + '_ {
        self.candidate_pairs(clearance_nm).filter_map(move |(i, j)| self.evaluate(i, j, clearance_nm))
    }
    /// DRC needs only clearance decisions; skip its existing exemptions before narrow-phase
    /// work, while the general query continues to expose contact independently for shorts.
    /// Across different islands, unresolved membership remains a clearance obstacle even when
    /// display names match: only one equal, resolved intended net proves a same-net exemption.
    pub(super) fn clearance_pairs(&self, clearance_nm: i64) -> impl Iterator<Item = Decision> + '_ {
        self.candidate_pairs(clearance_nm)
            .filter(move |(i, j)| {
                let (a, b) = (&self.items[*i], &self.items[*j]);
                a.island != b.island && !(a.intended_nets.len() == 1 && a.intended_nets == b.intended_nets)
            })
            .filter_map(move |(i, j)| self.evaluate(i, j, clearance_nm))
    }
    /// Evaluate shape separation independently of the candidate-enumeration strategy.
    pub(super) fn evaluate(&self, i: usize, j: usize, clearance_nm: i64) -> Option<Decision> {
        let (a, b) = (&self.items[i], &self.items[j]);
        share_layers(&a.layers, &b.layers)
            .then(|| decision(Some(i), j, a.contour.gap(&b.contour), &a.intended_nets, &b.intended_nets, a.island == b.island, clearance_nm))
    }
}

/// Unknown membership never proves same-net clearance exemption. Conflicted islands retain all
/// memberships for short decisions; even two equal conflicting sets are not one intended net.
fn decision(first: Option<usize>, second: usize, gap_nm: f64, a: &[usize], b: &[usize], same_island: bool, clearance_nm: i64) -> Decision {
    let same_net = a.len() == 1 && a == b;
    let conflicting_nets = gap_nm <= 0. && a.first().or_else(|| b.first()).is_some_and(|first| a.iter().chain(b).any(|net| net != first));
    Decision {
        first,
        second,
        gap_nm,
        touches: gap_nm <= 0.,
        clearance_violation: !same_island && !same_net && gap_nm < clearance_nm as f64,
        conflicting_nets,
    }
}

/// Layer sets are explicit: an empty set never invents an all-layer copper span.
fn share_layers(a: &[Uuid], b: &[Uuid]) -> bool {
    a.iter().any(|layer| b.contains(layer))
}

/// Inclusive expanded AABB comparison. False positives are allowed; final predicates decide.
fn near_bounds(a: [f64; 4], b: [f64; 4], clearance: f64) -> bool {
    a[0] <= b[2] + clearance && b[0] <= a[2] + clearance && a[1] <= b[3] + clearance && b[1] <= a[3] + clearance
}

#[cfg(test)]
mod tests;
