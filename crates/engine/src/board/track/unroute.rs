//! Plan a directional pad-to-pad unroute without changing the source board.
use super::*;
use crate::board::{
    Board,
    ratsnest::Endpoint,
    view::{BoardTarget, CopperLayers, rectangle_points},
    work::yield_unit,
};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

mod index;

use index::Index;
use serde_json::Map;

type Prepared = Result<(Option<Project>, Value), CommandError>;

// Counts candidate items copied or checked and extra delta walks, so a regression can
// bound per-step work and verify unrelated nets are skipped. Calling test thread only.
#[cfg(test)]
std::thread_local! {
    static CANDIDATE_ITEMS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static DELTA_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
    let mut removed = Map::new();
    for (key, ids) in [("tracks", &tracks), ("vias", &vias)] {
        let mut report_ids = Vec::new();
        for id in ids {
            yield_unit().await;
            report_ids.push(Value::String(id.to_string()));
        }
        removed.insert(key.to_owned(), Value::Array(report_ids));
    }
    let candidate = delete_candidate(project, &tracks, &vias).await?;
    yield_unit().await;
    let dry_run = params["dry_run"].as_bool() == Some(true);
    let mut report = Map::new();
    report.insert("dry_run".into(), Value::Bool(dry_run));
    report.insert("removed".into(), Value::Object(removed));
    report.insert("requested".into(), json!({"from":params["from"],"to":params["to"]}));
    report.insert("unrouted".into(), Value::Array(unrouted));
    Ok((candidate.filter(|_| !dry_run), Value::Object(report)))
}

/// One unit of candidate construction, counted by the step-work regression.
async fn candidate_unit() {
    yield_unit().await;
    #[cfg(test)]
    CANDIDATE_ITEMS.with(|items| items.set(items.get() + 1));
}

/// Build the shared deletion command's candidate one item per unit. Retained copper gets the
/// same checks in the same order as `validate`, then removed locked copper gets the refusal
/// of `protect_locked`. Preview and apply both run all of it, so they refuse alike.
async fn delete_candidate(project: &Project, tracks: &[Uuid], vias: &[Uuid]) -> Result<Option<Project>, CommandError> {
    if tracks.is_empty() && vias.is_empty() {
        return Err(CommandError::InvalidParams("name at least one track or via to delete".into()));
    }
    let source = &*project.board;
    let mut gone = BTreeSet::new();
    for id in tracks.iter().chain(vias) {
        candidate_unit().await;
        gone.insert(*id);
    }
    let invalid = CommandError::InvalidParams;
    validate_counts(source.tracks.len().saturating_sub(tracks.len()), source.vias.len().saturating_sub(vias.len())).map_err(invalid)?;
    let copper = copper_layer_ids(project);
    let mut ids = BTreeSet::new();
    let mut locked = None;
    // Saved and edited boards keep ID order, so the retained subsequence normally stays sorted.
    let mut ordered = true;
    let mut kept_tracks: Vec<Track> = Vec::with_capacity(source.tracks.len());
    for track in &source.tracks {
        candidate_unit().await;
        if gone.contains(&track.id) {
            if track.locked && locked.is_none() {
                locked = Some(("Track", track.id));
            }
        } else {
            check_track(track, &copper, &mut ids).map_err(invalid)?;
            ordered &= kept_tracks.last().is_none_or(|last| last.id < track.id);
            kept_tracks.push(track.clone());
        }
    }
    let mut kept_vias: Vec<Via> = Vec::with_capacity(source.vias.len());
    for via in &source.vias {
        candidate_unit().await;
        if gone.contains(&via.id) {
            // As in `protect_locked`, a locked track is reported before any via.
            if via.locked && locked.is_none() {
                locked = Some(("Via", via.id));
            }
        } else {
            ordered &= kept_vias.last().is_none_or(|last| last.id < via.id);
            kept_vias.push(via.clone());
        }
    }
    check_via_layers(!kept_vias.is_empty(), &copper).map_err(invalid)?;
    for via in &kept_vias {
        candidate_unit().await;
        check_via(via, &mut ids).map_err(invalid)?;
    }
    // Like `validate`, store the candidate in ID order.
    if !ordered {
        kept_tracks.sort_by_key(|track| track.id);
        kept_vias.sort_by_key(|via| via.id);
    }
    // Every cut ID names an item of this board, so equal counts mean nothing was removed.
    if kept_tracks.len() == source.tracks.len() && kept_vias.len() == source.vias.len() {
        return Ok(None);
    }
    if let Some((kind, id)) = locked {
        return Err(super::locks::locked(kind, id));
    }
    let board = Board {
        zones: copy_items(&source.zones).await,
        ownership: source.ownership.clone(),
        baseline: source.baseline.clone(),
        assignments: copy_map(&source.assignments).await,
        outline: source.outline,
        drawings: copy_drawings(source.drawings.as_ref()).await,
        setup: source.setup.clone(),
        silkscreen: source.silkscreen,
        rules: source.rules.clone(),
        placement_origin: source.placement_origin.clone(),
        assembly_profiles: copy_items(&source.assembly_profiles).await,
        tracks: kept_tracks,
        vias: kept_vias,
        arcs: copy_items(&source.arcs).await,
        teardrops: copy_items(&source.teardrops).await,
        definitions: copy_map(&source.definitions).await,
        footprints: copy_items(&source.footprints).await,
        by_component: copy_map(&source.by_component).await,
    };
    let mut next = project.clone();
    next.board = Arc::new(board);
    Ok(Some(next))
}

/// Copy a board collection one element per unit.
async fn copy_items<T: Clone>(items: &[T]) -> Vec<T> {
    let mut copy = Vec::with_capacity(items.len());
    for item in items {
        candidate_unit().await;
        copy.push(item.clone());
    }
    copy
}

/// Copy a board index one entry per unit.
async fn copy_map<K: Clone + Ord, V: Clone>(map: &BTreeMap<K, V>) -> BTreeMap<K, V> {
    let mut copy = BTreeMap::new();
    for (key, value) in map {
        candidate_unit().await;
        copy.insert(key.clone(), value.clone());
    }
    copy
}

/// Copy authored board drawings, their layers, and their vertices one item per unit.
async fn copy_drawings(drawings: Option<&crate::board::drawing::Drawings>) -> Option<crate::board::drawing::Drawings> {
    let drawings = drawings?;
    let layers = copy_items(&drawings.layers).await;
    let mut items = Vec::with_capacity(drawings.items.len());
    for drawing in &drawings.items {
        candidate_unit().await;
        let points = copy_items(&drawing.points).await;
        items.push(crate::board::drawing::Drawing {
            id: drawing.id,
            layer: drawing.layer,
            points,
            closed: drawing.closed,
            stroke_nm: drawing.stroke_nm,
        });
    }
    Some(crate::board::drawing::Drawings { layers, items })
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
            let center = footprint.point(pad.center);
            lands.push(Land {
                key: (id.clone(), number.to_owned()),
                corners: rectangle_points(footprint, pad.center, pad.width_nm, pad.height_nm, pad.rotation),
                layers,
                net: None,
                plated_slot: pad.plated_slot.and_then(|slot| slot.resolve(center, footprint.rotation, footprint.bottom)),
                center,
                shape: pad.shape,
                corner_radius_nm: pad.corner_radius_nm,
                size_nm: (pad.width_nm, pad.height_nm),
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
    let Some(affected) = same_net(view.project(), [&lands[from], &lands[to]]).await else {
        return Err(CommandError::InvalidParams("Both pads must belong to the same intended net.".into()));
    };
    let contacts = Contacts::new(&lands, tracks, vias).await;
    let no_removals = vec![false; contacts.bounds.len()];
    let mut before = vec![None; no_removals.len()];
    let original = contacts.walk(to, &no_removals, &mut before).await;
    let Some(first) = before[from] else {
        return Err(CommandError::InvalidParams("The pads are not joined by routed copper.".into()));
    };
    // Routed islands don't count direct pad-on-pad overlaps, and no track cut separates a
    // pad from one it touches. Overlaps elsewhere get the post-cut connectivity checks.
    if !contacts.contacts(from, true).await.is_empty() {
        return Err(CommandError::InvalidParams("The pad to detach overlaps another pad; delete copper explicitly.".into()));
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
    let via_start = lands.len() + tracks.len();
    // An explicitly assigned via anchors its copper even on the selected path, so the cut
    // stops before it and keeps it with the copper on its far side.
    let assigned = |node: usize| node >= via_start && vias[node - via_start].net_assignment.is_some();
    let mut remove = no_removals.clone();
    let mut node = first;
    while node >= lands.len() && !protected[node] && !assigned(node) {
        yield_unit().await;
        remove[node] = true;
        let Some(parent) = before[node] else { break };
        node = parent;
    }
    if assigned(first) {
        return Err(CommandError::InvalidParams("An assigned via joins the pad to detach; it cannot be kept while separating them.".into()));
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
    // A pad overlap elsewhere can still join the detached side to the retained copper,
    // which routed walks don't cross. Follow such overlaps before claiming separation.
    let mut reached = no_removals.clone();
    reached[from] = true;
    let mut queue = vec![from];
    let mut i = 0;
    while let Some(&node) = queue.get(i) {
        yield_unit().await;
        i += 1;
        let mut neighbors = contacts.contacts(node, false).await;
        if node < lands.len() && node != from {
            neighbors.extend(contacts.contacts(node, true).await);
        }
        for neighbor in neighbors {
            yield_unit().await;
            if remove[neighbor] || reached[neighbor] {
                continue;
            }
            if after[neighbor].is_some() {
                return Err(CommandError::InvalidParams("Cannot separate these pads: they stay joined through overlapping pads.".into()));
            }
            reached[neighbor] = true;
            queue.push(neighbor);
        }
    }

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
            // Assigned vias and the copper beyond them stay for the anchor check below.
            if neighbor >= lands.len() && !assigned(neighbor) && from_side[neighbor].take().is_some() {
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
    let unrouted = airwire_delta(view, &contacts, &lands, (from, to), &affected, &no_removals, &mut before).await;
    Ok(Cut { tracks: track_ids, vias: via_ids, unrouted })
}

/// Search only for the two mapped terminals; the accepted baseline keeps last-match precedence.
/// When they share a net, return every accepted net containing the first terminal, in index
/// order: only their display can change when its pad is detached. A pad belongs to one
/// terminal, so no other net resolves an endpoint there. Without an accepted baseline,
/// retain the existing authoritative electrical graph fallback, which displays no airwires.
async fn same_net(project: &Project, lands: [&Land; 2]) -> Option<Vec<usize>> {
    let terminals = lands.map(|land| {
        let pad = super::super::Pad::parse(&land.key.0, &land.key.1)?;
        project.board.component_pin_for(pad)
    });
    let [Some(a), Some(b)] = terminals else { return None };
    if let Some(baseline) = &project.board.baseline {
        let mut membership = [None; 2];
        let mut affected = Vec::new();
        for (index, net) in baseline.nets.iter().enumerate() {
            yield_unit().await;
            for terminal in &net.terminals {
                yield_unit().await;
                let key = (terminal.component, terminal.terminal.as_str());
                if key == a {
                    membership[0] = Some(index);
                    if affected.last() != Some(&index) {
                        affected.push(index);
                    }
                }
                if key == b {
                    membership[1] = Some(index);
                }
            }
        }
        (membership[0].is_some() && membership[0] == membership[1]).then_some(affected)
    } else {
        yield_unit().await;
        let graph = project.borrowed_electrical_connectivity();
        let [a, b] = [a, b].map(|(component, pin)| project.component_terminal(component, pin).and_then(|end| graph.net_index_for(end)));
        (a.is_some() && a == b).then(Vec::new)
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
    index: Index,
    pads: usize,
}
impl Contacts {
    /// Project one geometry node at a time, then build the global hierarchy cooperatively.
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
        let index = Index::new(&bounds).await;
        Self { contours, bounds, layers, index, pads: lands.len() }
    }

    /// Prune disjoint subtrees and decide exact contacts in increasing source-ID order.
    /// Direct pad overlap remains strict; routed contacts remain inclusive.
    async fn contacts(&self, node: usize, pads_only: bool) -> Vec<usize> {
        let mut result = Vec::new();
        for other in self.index.query(self.bounds[node]).await {
            yield_unit().await;
            if other == node || (pads_only && other >= self.pads) {
                continue;
            }
            let touching = if pads_only {
                self.layers[node].iter().any(|layer| self.layers[other].contains(layer)) && self.contours[node].overlaps(&self.contours[other])
            } else {
                node.max(other) >= self.pads && routed_contact(&self.layers, &self.bounds, &self.contours, node.max(other), node.min(other))
            };
            if touching {
                result.push(other);
            }
        }
        result
    }

    /// Mark and return one component in breadth-first order. Reusing marks avoids
    /// revisiting components while enumerating disconnected pads for the report.
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
/// orphan components touch no pads. All other islands keep their original membership, so
/// only the `affected` nets, which contain the detached pad, are built before and after.
async fn airwire_delta(
    view: BoardView<'_>,
    contacts: &Contacts,
    lands: &[Land],
    (from, to): (usize, usize),
    affected: &[usize],
    no_removals: &[bool],
    parent: &mut [Option<usize>],
) -> Vec<Value> {
    if affected.is_empty() || !view.has_accepted_connectivity() {
        return Vec::new();
    }
    let Some(baseline) = &view.project().board.baseline else {
        return Vec::new();
    };
    let mut affected_terminals = BTreeSet::new();
    for &net in affected {
        yield_unit().await;
        let Some(state) = baseline.nets.get(net) else { continue };
        for terminal in &state.terminals {
            yield_unit().await;
            affected_terminals.insert((terminal.component, terminal.terminal.as_str()));
        }
    }
    // Canonical display islands depend only on equality, not on the numeric IDs.
    let mut copper = Copper::default();
    for (node, land) in lands.iter().enumerate() {
        yield_unit().await;
        if parent.get(node).copied().flatten().is_some() {
            // Before any other walk, these are precisely the original component.
            copper.pads.insert(land.key.clone(), to);
        }
    }
    for (node, land) in lands.iter().enumerate() {
        yield_unit().await;
        let in_affected = super::super::Pad::parse(&land.key.0, &land.key.1)
            .and_then(|pad| view.project().board.component_pin_for(pad))
            .is_some_and(|terminal| affected_terminals.contains(&terminal));
        if in_affected && parent.get(node).copied().flatten().is_none() {
            #[cfg(test)]
            DELTA_WALKS.with(|walks| walks.set(walks.get() + 1));
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
    for line in view.build_ratsnest::<true>(&copper, Some(affected)).await.into_iter().flat_map(|r| r.lines) {
        yield_unit().await;
        old.insert((line.net, edge(&line.from, &line.to)));
    }
    if let Some(land) = lands.get(from) {
        copper.pads.insert(land.key.clone(), lands.len());
    }
    let address = |end: &Endpoint| json!({"footprint":end.footprint,"pad":end.pad.number.as_str()});
    let mut delta = Vec::new();
    for line in view.build_ratsnest::<true>(&copper, Some(affected)).await.into_iter().flat_map(|r| r.lines) {
        yield_unit().await;
        if !old.contains(&(line.net, edge(&line.from, &line.to))) {
            delta.push(json!({"from":address(&line.from),"to":address(&line.to)}));
        }
    }
    delta
}

#[cfg(test)]
mod tests;
