//! Plan a directional pad-to-pad unroute without changing the source board.
use super::*;
use crate::board::{clearance::index::Tree, ratsnest::Endpoint, view::BoardTarget};

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
        while !job.step(usize::MAX) {}
        let prepared = job.take_result()?.ok_or_else(|| CommandError::Unavailable("Unroute preparation returned no result.".into()))?;
        self.commit_copper_edit_with_report(prepared)?.1.ok_or_else(|| CommandError::Unavailable("Unroute preparation returned no report.".into()))
    }
}

/// Plan and validate deletion against the captured board. A preview checks the same locks but
/// does not send a candidate for publication. No connectivity work runs during publication.
pub(super) fn prepare(project: &Project, target: BoardTarget, params: &Value) -> Result<(Option<Project>, Value), CommandError> {
    let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
    let Cut { tracks, vias, unrouted } = plan(view, &params["from"], &params["to"])?;
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
fn plan(view: BoardView<'_>, from: &Value, to: &Value) -> Result<Cut, CommandError> {
    let lands = lands(view);
    let endpoint = |value: &Value| {
        let footprint = value["footprint"].as_str()?;
        let pad = value["pad"].as_str()?;
        lands.iter().position(|land| land.key.0 == footprint && land.key.1 == pad)
    };
    let (Some(from), Some(to)) = (endpoint(from), endpoint(to)) else {
        return Err(CommandError::NotFound("Both endpoints must be pads with copper on the addressed board.".into()));
    };
    if from == to {
        return Err(CommandError::InvalidParams("Choose two distinct pads.".into()));
    }
    let (tracks, vias) = (view.tracks(), view.vias());
    let endpoints = [lands[from].clone(), lands[to].clone()];
    let membership = super::super::via_nets::node_membership(view.project(), &endpoints, 0, &[], &Default::default());
    if membership[0].is_none() || membership[0] != membership[1] {
        return Err(CommandError::InvalidParams("Both pads must belong to the same intended net.".into()));
    }
    let contacts = Contacts::new(&lands, tracks, vias);
    let no_removals = vec![false; contacts.geometry.bounds.len()];
    let mut before = vec![None; no_removals.len()];
    let original = contacts.walk(to, &no_removals, &mut before);
    let Some(first) = before[from] else {
        return Err(CommandError::InvalidParams("The pads are not joined by routed copper.".into()));
    };
    // Routed islands don't count direct pad-on-pad overlaps. Refuse an arm whose island
    // touches one rather than claiming physical separation from the ratsnest alone.
    let mut direct_land_contact = false;
    for a in original.iter().copied().filter(|&node| node < lands.len()) {
        let g = &contacts.geometry;
        direct_land_contact |= contacts.index.candidates(g.bounds[a], 0.).into_iter().any(|b| {
            b < lands.len() && b != a && g.layers[a].iter().any(|layer| g.layers[b].contains(layer)) && g.contours[a].overlaps(&g.contours[b])
        });
    }
    if direct_land_contact {
        return Err(CommandError::InvalidParams("The routed island has overlapping pads; delete copper explicitly.".into()));
    }
    let mut protected = no_removals.clone();
    for pad in (0..lands.len()).filter(|&pad| pad != from) {
        let mut node = pad;
        while let Some(parent) = before[node] {
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
        remove[node] = true;
        let Some(parent) = before[node] else { break };
        node = parent;
    }
    if !remove.iter().any(|&removed| removed) {
        return Err(CommandError::InvalidParams("No exclusive routed arm leaves the first pad.".into()));
    }
    let mut after = vec![None; remove.len()];
    contacts.walk(to, &remove, &mut after);
    if after[from].is_some() || (0..lands.len()).any(|pad| pad != from && before[pad].is_some() && after[pad].is_none()) {
        return Err(CommandError::InvalidParams("Cannot separate these pads without cutting another connection.".into()));
    }

    let via_start = lands.len() + tracks.len();
    // Contour overlap at the pad can let BFS skip part of a segmented arm. Follow
    // the cut into the now-exclusive from side, without crossing the pad onto
    // unrelated copper that only used to touch the arm through that pad.
    let mut from_side = vec![None; remove.len()];
    contacts.walk(from, &remove, &mut from_side);
    let mut stubs: Vec<_> = original.iter().copied().filter(|&node| remove[node]).collect();
    let mut i = 0;
    while let Some(&node) = stubs.get(i) {
        i += 1;
        for neighbor in contacts.neighbors(node) {
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
    let cut: Vec<_> = original.into_iter().filter(|&node| remove[node]).collect();
    for node in cut.into_iter().flat_map(|node| contacts.neighbors(node)) {
        if !remove[node] && after[node].is_none() {
            let component = contacts.walk(node, &remove, &mut after);
            // Pads and retained locked or explicitly assigned vias anchor the group.
            // A locked track in any required removal is refused by shared deletion.
            if component
                .iter()
                .all(|&n| n >= lands.len() && (n < via_start || (!vias[n - via_start].locked && vias[n - via_start].net_assignment.is_none())))
            {
                for n in component {
                    remove[n] = true;
                }
            }
        }
    }

    let tracks = tracks.iter().enumerate().filter(|(i, _)| remove[lands.len() + i]).map(|(_, track)| track.id).collect();
    let vias = vias.iter().enumerate().filter(|(i, _)| remove[via_start + i]).map(|(_, via)| via.id).collect();
    let unrouted = airwire_delta(view, &contacts, &lands, (from, to), &no_removals, &mut before);
    Ok(Cut { tracks, vias, unrouted })
}

/// Indexed contour contacts, with the same predicate and stable order as routed islands.
struct Contacts {
    geometry: IslandBuilder,
    index: Tree,
    pads: usize,
}
impl Contacts {
    /// Build one index for all searches, including searches with a deletion mask.
    fn new(lands: &[Land], tracks: &[Track], vias: &[Via]) -> Self {
        let geometry = IslandBuilder::new(lands, tracks, vias);
        let index = Tree::new(geometry.bounds.clone());
        Self { geometry, index, pads: lands.len() }
    }

    /// Enumerate only envelope candidates, filtering pad-only contacts and layer mismatches.
    fn neighbors(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        self.index.candidates(self.geometry.bounds[node], 0.).into_iter().filter(move |&neighbor| {
            node != neighbor && node.max(neighbor) >= self.pads && self.geometry.routed_contact(node.max(neighbor), node.min(neighbor))
        })
    }

    /// Mark and return one component in breadth-first order. Reuse visitation marks for
    /// disjoint components so enumerating disconnected pads stays proportional to the graph.
    fn walk(&self, root: usize, remove: &[bool], parent: &mut [Option<usize>]) -> Vec<usize> {
        let mut queue = vec![root];
        parent[root] = Some(root);
        let mut i = 0;
        while let Some(&node) = queue.get(i) {
            i += 1;
            for neighbor in self.neighbors(node) {
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
fn airwire_delta(
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
        if parent[node].is_some() {
            // Before any other walk, these are precisely the original component.
            copper.pads.insert(land.key.clone(), to);
        }
    }
    for node in 0..lands.len() {
        if parent[node].is_none() {
            for member in contacts.walk(node, no_removals, parent) {
                if let Some(land) = lands.get(member) {
                    copper.pads.insert(land.key.clone(), node);
                }
            }
        }
    }
    let edge = |from: &Endpoint, to: &Endpoint| (from.pad.footprint, from.pad.number, to.pad.footprint, to.pad.number);
    let old: BTreeSet<_> =
        view.ratsnest_with_copper(&copper).into_iter().flat_map(|r| r.lines).map(|line| (line.net, edge(&line.from, &line.to))).collect();
    copper.pads.insert(lands[from].key.clone(), lands.len());
    let address = |end: &Endpoint| json!({"footprint":end.footprint,"pad":end.pad.number.as_str()});
    view.ratsnest_with_copper(&copper)
        .into_iter()
        .flat_map(|r| r.lines)
        .filter(|line| !old.contains(&(line.net, edge(&line.from, &line.to))))
        .map(|line| json!({"from":address(&line.from),"to":address(&line.to)}))
        .collect()
}

#[cfg(test)]
mod tests;
