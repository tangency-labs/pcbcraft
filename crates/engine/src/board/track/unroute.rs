//! Plan a directional pad-to-pad unroute without changing the source board.
use super::*;
use crate::board::{
    clearance::index::Tree,
    ratsnest::Endpoint,
    view::{BoardTarget, CopperLayers, rectangle_points},
    work::yield_unit,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

type Prepared = Result<(Option<Project>, Value), CommandError>;

/// Own the immutable inputs while any native or browser work queue advances the planner.
/// Dropping the continuation cancels all remaining work without producing a candidate.
pub(super) struct Planner {
    work: Pin<Box<dyn Future<Output = Prepared> + Send>>,
    completed: usize,
}

impl Planner {
    /// Capture only owned inputs; the first planning unit runs when polled.
    pub(super) fn new(snapshot: std::sync::Arc<Project>, target: BoardTarget, params: Value) -> Self {
        Self { work: Box::pin(async move { prepare(&snapshot, target, &params).await }), completed: 0 }
    }

    /// Consume at most `budget` units, retaining traversal state between polls.
    pub(super) fn step(&mut self, budget: usize) -> Option<Prepared> {
        let mut context = Context::from_waker(Waker::noop());
        for _ in 0..budget {
            match self.work.as_mut().poll(&mut context) {
                Poll::Ready(result) => return Some(result),
                Poll::Pending => self.completed = self.completed.saturating_add(1),
            }
        }
        None
    }

    /// Logical progress, not an estimate of total work on an overlapping board.
    pub(super) fn completed(&self) -> usize {
        self.completed
    }
}

/// Remove the copper arm leaving one pad, keeping the connections between other pads.
pub const UNROUTE_CONNECTION: &str = "pcb.unroute_connection";

/// Describe a land address local to the explicitly addressed board.
pub(crate) fn pad_address_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["footprint","pad"],"properties":{
        "footprint":crate::ParamKind::FootprintId.schema(),"pad":{"type":"string","pattern":"^[A-Za-z0-9+_-]{1,8}$"}
    }})
}

/// Parse a strict local land address; actual copper must still be resolved on the board.
pub(crate) fn pad_address(value: &Value) -> Option<super::super::Pad> {
    if value.as_object()?.len() != 2 {
        return None;
    }
    super::super::Pad::parse(value["footprint"].as_str()?, value["pad"].as_str()?)
}

impl Session {
    /// Plan against the requested revision, validate the same deletion for preview or apply,
    /// and publish at most one authored change, through the same job as the live editor.
    pub(crate) fn unroute_connection(&mut self, params: &Value) -> Result<Value, CommandError> {
        let mut job = self.prepare_copper_edit(UNROUTE_CONNECTION, params)?;
        while !job.step(1024) {}
        let prepared = job.take_result()?.ok_or_else(|| CommandError::Unavailable("Unroute preparation returned no result.".into()))?;
        self.commit_copper_edit_with_report(prepared)?.1.ok_or_else(|| CommandError::Unavailable("Unroute preparation returned no report.".into()))
    }
}

/// Plan and validate deletion against the captured board. A preview checks the same locks but
/// does not send a candidate for publication. No connectivity work runs during publication.
async fn prepare(project: &Project, target: BoardTarget, params: &Value) -> Prepared {
    let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
    let Cut { tracks, vias, unrouted } = plan(view, &params["from"], &params["to"]).await?;
    yield_unit().await;
    let mut deletion = json!({});
    if !tracks.is_empty() {
        deletion["tracks"] = json!(tracks);
    }
    if !vias.is_empty() {
        deletion["vias"] = json!(vias);
    }
    let (candidate, _) = edit_candidate(project, DELETE_TRACKS, &deletion)?;
    let dry_run = params["dry_run"].as_bool() == Some(true);
    Ok((
        candidate.filter(|_| !dry_run),
        json!({"dry_run":dry_run,"removed":{"tracks":tracks,"vias":vias},
        "requested":{"from":params["from"],"to":params["to"]},"unrouted":unrouted}),
    ))
}

struct Cut {
    tracks: Vec<Uuid>,
    vias: Vec<Uuid>,
    unrouted: Vec<Value>,
}

/// Find a removable arm on a stable shortest contact path, reserving the paths back to `to`
/// from every other pad. Refuse alternate routes or collateral breaks instead of cutting more
/// than the selected arm. Validate before the deletion code sees any IDs.
async fn plan(view: BoardView<'_>, from: &Value, to: &Value) -> Result<Cut, CommandError> {
    let mut lands = Vec::new();
    let addresses = [from, to].map(|value| (value["footprint"].as_str(), value["pad"].as_str()));
    let mut endpoints = [None; 2];
    let project = view.project();
    for footprint in project.board.footprints() {
        yield_unit().await;
        // Match the resolved footprint view, but skip graphics and unused display-net names.
        if !project.components.contains_key(&footprint.component)
            && !project.board.baseline.as_ref().is_some_and(|baseline| baseline.components.contains_key(&footprint.component))
        {
            continue;
        }
        let Some(definition) = project.board.definition_for(footprint) else { continue };
        let id = format!("footprint-{}", footprint.id);
        for pad in &definition.pads {
            yield_unit().await;
            let Some(number) = crate::schematic::PinNumber::new(&pad.number) else { continue };
            let number = number.as_str();
            let CopperLayers::Resolved { layers, .. } = CopperLayers::resolve(view.setup(), pad.copper, footprint.bottom) else { continue };
            for (endpoint, address) in endpoints.iter_mut().zip(addresses) {
                if address == (Some(id.as_str()), Some(number)) {
                    endpoint.get_or_insert(lands.len());
                }
            }
            lands.push(Land {
                key: (id.clone(), number.to_owned()),
                corners: rectangle_points(footprint, pad.center, pad.width_nm, pad.height_nm, pad.rotation),
                layers,
                net: None,
                center: footprint.point(pad.center),
                shape: pad.shape,
                corner_radius_nm: pad.corner_radius_nm,
            });
        }
    }
    let [Some(from), Some(to)] = endpoints else {
        return Err(CommandError::NotFound("Both endpoints must be pads with copper on the addressed board.".into()));
    };
    if from == to {
        return Err(CommandError::InvalidParams("Choose two distinct pads.".into()));
    }
    let (tracks, vias) = (view.tracks(), view.vias());
    if !same_net(view.project(), [&lands[from], &lands[to]]).await {
        return Err(CommandError::InvalidParams("Both pads must belong to the same intended net.".into()));
    }
    let contacts = Contacts::new(&lands, tracks, vias).await;
    let no_removals = vec![false; contacts.bounds.len()];
    let mut before = vec![None; no_removals.len()];
    let original = contacts.walk(to, &no_removals, &mut before).await;
    let Some(first) = before[from] else {
        return Err(CommandError::InvalidParams("The pads are not joined by routed copper.".into()));
    };
    // Routed islands don't count direct pad-on-pad overlaps. Refuse an arm whose island
    // touches one rather than claiming physical separation from the ratsnest alone.
    for &a in &original {
        yield_unit().await;
        if a < lands.len() && !contacts.contacts(a, true).await.is_empty() {
            return Err(CommandError::InvalidParams("The routed island has overlapping pads; delete copper explicitly.".into()));
        }
    }
    let mut protected = no_removals.clone();
    for pad in (0..lands.len()).filter(|&pad| pad != from) {
        yield_unit().await;
        let mut node = pad;
        while let Some(parent) = before[node] {
            yield_unit().await;
            if protected[node] {
                break;
            }
            protected[node] = true;
            node = parent;
        }
    }
    let mut remove = no_removals.clone();
    let mut node = first;
    while node >= lands.len() && !protected[node] {
        yield_unit().await;
        remove[node] = true;
        let Some(parent) = before[node] else { break };
        node = parent;
    }
    if !remove[first] {
        return Err(CommandError::InvalidParams("No exclusive routed arm leaves the first pad.".into()));
    }
    let mut after = vec![None; remove.len()];
    contacts.walk(to, &remove, &mut after).await;
    for pad in 0..lands.len() {
        yield_unit().await;
        let retained = if pad == from { after[pad].is_none() } else { before[pad].is_none() || after[pad].is_some() };
        if !retained {
            return Err(CommandError::InvalidParams("Cannot separate these pads without cutting another connection.".into()));
        }
    }

    let via_start = lands.len() + tracks.len();
    // Contour overlap at the pad can let BFS skip part of a segmented arm. Follow
    // the cut into the now-exclusive from side, without crossing the pad onto
    // unrelated copper that only used to touch the arm through that pad.
    let mut from_side = vec![None; remove.len()];
    contacts.walk(from, &remove, &mut from_side).await;
    let mut stubs = cut_frontier(&original, &remove).await;
    let mut i = 0;
    while let Some(&node) = stubs.get(i) {
        i += 1;
        for neighbor in contacts.contacts(node, false).await {
            yield_unit().await;
            if neighbor >= lands.len() && from_side[neighbor].take().is_some() {
                stubs.push(neighbor);
                if neighbor < via_start {
                    remove[neighbor] = true;
                }
            }
        }
    }

    // End caps can also shortcut a segment between two cut segments. Only walk
    // residual components touching the cut; copper isolated before it is unrelated.
    // The retained side is already marked and none of the stub removals touched it.
    for cut in cut_frontier(&original, &remove).await {
        for node in contacts.contacts(cut, false).await {
            yield_unit().await;
            if remove[node] || after[node].is_some() {
                continue;
            }
            let component = contacts.walk(node, &remove, &mut after).await;
            // Retained pads and locked or explicitly assigned vias anchor the group.
            // A required locked track is refused by shared deletion.
            let mut anchored = false;
            for &n in &component {
                yield_unit().await;
                if n < lands.len() || (n >= via_start && (vias[n - via_start].locked || vias[n - via_start].net_assignment.is_some())) {
                    anchored = true;
                    break;
                }
            }
            if !anchored {
                for n in component {
                    yield_unit().await;
                    remove[n] = true;
                }
            }
        }
    }

    let mut track_ids = Vec::new();
    let mut via_ids = Vec::new();
    for (i, track) in tracks.iter().enumerate() {
        yield_unit().await;
        if remove[lands.len() + i] {
            track_ids.push(track.id);
        }
    }
    for (i, via) in vias.iter().enumerate() {
        yield_unit().await;
        if remove[via_start + i] {
            via_ids.push(via.id);
        }
    }
    let unrouted = airwire_delta(view, &contacts, &lands, (from, to), &no_removals, &mut before).await;
    Ok(Cut { tracks: track_ids, vias: via_ids, unrouted })
}

/// Search only for the two mapped terminals; the accepted baseline keeps last-match precedence.
/// Without an accepted baseline, retain the existing authoritative electrical graph fallback.
async fn same_net(project: &Project, lands: [&Land; 2]) -> bool {
    let terminals = lands.map(|land| {
        let pad = super::super::Pad::parse(&land.key.0, &land.key.1)?;
        project.board.component_pin_for(pad)
    });
    let [Some(a), Some(b)] = terminals else { return false };
    if let Some(baseline) = &project.board.baseline {
        let mut membership = [None; 2];
        for (index, net) in baseline.nets.iter().enumerate() {
            yield_unit().await;
            for terminal in &net.terminals {
                yield_unit().await;
                let key = (terminal.component, terminal.terminal.as_str());
                if key == a {
                    membership[0] = Some(index);
                }
                if key == b {
                    membership[1] = Some(index);
                }
            }
        }
        membership[0].is_some() && membership[0] == membership[1]
    } else {
        yield_unit().await;
        let graph = project.borrowed_electrical_connectivity();
        let [a, b] = [a, b].map(|(component, pin)| project.component_terminal(component, pin).and_then(|end| graph.net_index_for(end)));
        a.is_some() && a == b
    }
}

/// Collect removed nodes in the original component's breadth-first order.
async fn cut_frontier(original: &[usize], remove: &[bool]) -> Vec<usize> {
    let mut cut = Vec::new();
    for &node in original {
        yield_unit().await;
        if remove[node] {
            cut.push(node);
        }
    }
    cut
}

/// Indexed contour contacts, with the same predicate and stable order as routed islands.
struct Contacts {
    contours: Vec<PadContour>,
    bounds: Vec<[f64; 4]>,
    layers: Vec<Vec<Uuid>>,
    index: Vec<Tree>,
    pads: usize,
}
impl Contacts {
    /// Each packed block bounds index construction and lookup to 128 envelopes. Blocks and
    /// their hits are visited in source order, preserving shortest-path tie breaking.
    const BLOCK: usize = 128;

    /// Project one geometry node at a time, then index one fixed-size block per unit.
    async fn new(lands: &[Land], tracks: &[Track], vias: &[Via]) -> Self {
        let mut contours = Vec::new();
        let mut bounds = Vec::new();
        let mut layers = Vec::new();
        for land in lands {
            yield_unit().await;
            contours.push(land.contour());
            layers.push(land.layers.clone());
        }
        for track in tracks {
            yield_unit().await;
            contours.push(PadContour::capsule(track.start, track.end, track.width_nm as f64 / 2.));
            layers.push(vec![track.layer]);
        }
        for via in vias {
            yield_unit().await;
            contours.push(PadContour::capsule(via.at, via.at, via.diameter_nm as f64 / 2.));
        }
        for contour in &contours {
            yield_unit().await;
            bounds.push(contour.bounds());
        }
        let mut index = Vec::new();
        for block in bounds.chunks(Self::BLOCK) {
            yield_unit().await;
            index.push(Tree::new(block.to_vec()));
        }
        Self { contours, bounds, layers, index, pads: lands.len() }
    }

    /// Query one bounded block, then check each candidate on its own work unit. Pad-only
    /// checks use strict overlap; routed checks use the legacy inclusive contact predicate.
    async fn contacts(&self, node: usize, pads_only: bool) -> Vec<usize> {
        let mut result = Vec::new();
        for (block, index) in self.index.iter().enumerate() {
            yield_unit().await;
            let start = block * Self::BLOCK;
            if pads_only && start >= self.pads {
                break;
            }
            for candidate in index.candidates(self.bounds[node], 0.) {
                yield_unit().await;
                let other = start + candidate;
                let touching = if pads_only {
                    other < self.pads
                        && self.layers[node].iter().any(|layer| self.layers[other].contains(layer))
                        && self.contours[node].overlaps(&self.contours[other])
                } else {
                    node.max(other) >= self.pads && routed_contact(&self.layers, &self.bounds, &self.contours, node.max(other), node.min(other))
                };
                if other != node && touching {
                    result.push(other);
                }
            }
        }
        result
    }

    /// Mark and return one component in breadth-first order. Reuse visitation marks for
    /// disjoint components so enumerating disconnected pads stays proportional to the graph.
    async fn walk(&self, root: usize, remove: &[bool], parent: &mut [Option<usize>]) -> Vec<usize> {
        let mut queue = vec![root];
        parent[root] = Some(root);
        let mut i = 0;
        while let Some(&node) = queue.get(i) {
            yield_unit().await;
            i += 1;
            for neighbor in self.contacts(node, false).await {
                yield_unit().await;
                if !remove[neighbor] && parent[neighbor].is_none() {
                    parent[neighbor] = Some(node);
                    queue.push(neighbor);
                }
            }
        }
        queue
    }
}

/// Reuse the accepted display's ordering rather than inventing an airwire from the requested
/// endpoints. Only `from` changes pad connectivity: stubs cannot touch the retained side and
/// orphan components touch no pads. All other islands keep their original membership.
async fn airwire_delta(
    view: BoardView<'_>,
    contacts: &Contacts,
    lands: &[Land],
    (from, to): (usize, usize),
    no_removals: &[bool],
    parent: &mut [Option<usize>],
) -> Vec<Value> {
    if !view.has_accepted_connectivity() {
        return Vec::new();
    }
    // Canonical display islands depend only on equality, not on the numeric IDs.
    let mut copper = Copper::default();
    for (node, land) in lands.iter().enumerate() {
        yield_unit().await;
        if parent[node].is_some() {
            // Before any other walk, these are precisely the original component.
            copper.pads.insert(land.key.clone(), to);
        }
    }
    for node in 0..lands.len() {
        yield_unit().await;
        if parent[node].is_none() {
            for member in contacts.walk(node, no_removals, parent).await {
                yield_unit().await;
                if let Some(land) = lands.get(member) {
                    copper.pads.insert(land.key.clone(), node);
                }
            }
        }
    }
    let edge = |from: &Endpoint, to: &Endpoint| (from.pad.footprint, from.pad.number, to.pad.footprint, to.pad.number);
    let mut old = BTreeSet::new();
    for line in view.build_ratsnest::<true>(&copper).await.into_iter().flat_map(|r| r.lines) {
        yield_unit().await;
        old.insert((line.net, edge(&line.from, &line.to)));
    }
    copper.pads.insert(lands[from].key.clone(), lands.len());
    let address = |end: &Endpoint| json!({"footprint":end.footprint,"pad":end.pad.number.as_str()});
    let mut delta = Vec::new();
    for line in view.build_ratsnest::<true>(&copper).await.into_iter().flat_map(|r| r.lines) {
        yield_unit().await;
        if !old.contains(&(line.net, edge(&line.from, &line.to))) {
            delta.push(json!({"from":address(&line.from),"to":address(&line.to)}));
        }
    }
    delta
}

#[cfg(test)]
mod tests;
