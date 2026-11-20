//! Copper track segments and through vias on configured copper layers, and the copper
//! connectivity they create.
//!
//! A track is one straight segment with an explicit width on one copper layer. A via is a
//! plated hole whose round pad spans every copper layer. Routing a path adds its segments and
//! vias in one undo step. Connectivity is geometric: copper on the same layer connects where it
//! overlaps, and a via joins whatever copper it overlaps on any layer. This is a bounded connectivity aid for the ratsnest and for
//! refusing shorts between nets. It is not a clearance check, DRC or manufacturing validation.
#[cfg(test)]
use super::geometry::{inside, segment_distance};
use super::{
    definition::PadShape,
    geometry::Contour as PadContour,
    pair_index::{Hit, PairIndex, Search},
    setup::LayerPurpose,
    view::{BoardView, CopperLayers},
};
use crate::{CommandError, Project, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Add a routed path of straight track segments, with any vias where it changes layer, in one undo step.
pub const ADD_TRACKS: &str = "pcb.add_tracks";
/// Place one standalone through via, for stitching or to start a route, in one undo step.
pub const ADD_VIA: &str = "pcb.add_via";
/// Move addressed tracks and vias by one displacement, keeping their sizes and layers.
pub const MOVE_TRACKS: &str = "pcb.move_tracks";
/// Set new ends for addressed track segments at once, keeping their layers and widths.
pub const RESHAPE_TRACKS: &str = "pcb.reshape_tracks";
/// Change the width of addressed track segments.
pub const SET_TRACK_WIDTH: &str = "pcb.set_track_width";
/// Change the pad diameter and drill of addressed vias.
pub const SET_VIA_SIZE: &str = "pcb.set_via_size";
/// Remove addressed track segments and vias without changing footprints or other copper.
pub const DELETE_TRACKS: &str = "pcb.delete_tracks";
mod unroute;
pub use unroute::UNROUTE_CONNECTION;
pub(crate) use unroute::{pad_address, pad_address_schema};
/// Lock or unlock explicitly addressed tracks and vias without changing geometry.
pub const SET_COPPER_LOCKED: &str = "pcb.set_copper_locked";

mod locks;
pub mod preparation;
pub(crate) use locks::protect_locked;
#[cfg(test)]
mod capacity_tests;
#[cfg(test)]
mod lock_tests;

/// The width the PCB route tool starts with, 0.25 mm.
pub const DEFAULT_WIDTH_NM: i64 = 250_000;
/// Narrowest accepted track, 0.01 mm. Manufacturability is a separate, later check.
pub const MIN_WIDTH_NM: i64 = 10_000;
/// Widest accepted track, 10 mm.
pub const MAX_WIDTH_NM: i64 = 10_000_000;
/// Vertices accepted for one routed path.
pub const MAX_PATH_POINTS: usize = 256;
/// Track segments accepted on one board and in a command's track list. Clipboard text and
/// routing work have separate limits; this count does not bound island-check latency.
pub const MAX_TRACKS: usize = 65_536;
/// Vias accepted on one board.
pub const MAX_VIAS: usize = 16_384;
/// The via pad diameter the route tool starts with, 0.6 mm.
pub const DEFAULT_VIA_DIAMETER_NM: i64 = 600_000;
/// The via drill the route tool starts with, 0.3 mm.
pub const DEFAULT_VIA_DRILL_NM: i64 = 300_000;
/// Smallest accepted via drill, 0.05 mm. Manufacturability is a separate, later check.
pub const MIN_VIA_DRILL_NM: i64 = 50_000;
/// Largest accepted via pad diameter, 10 mm.
pub const MAX_VIA_DIAMETER_NM: i64 = 10_000_000;

/// One straight copper segment. Its ends are round, so copper extends `width_nm / 2` past each end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub id: Uuid,
    /// Fixed copper refuses geometry, size and deletion edits until explicitly unlocked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    pub layer: Uuid,
    pub width_nm: i64,
    pub start: Point,
    pub end: Point,
}

impl Track {
    /// Inspection always exposes lock state, including the default false omitted from native files.
    pub fn inspect(&self) -> Value {
        let mut value = json!(self);
        value["locked"] = json!(self.locked);
        value
    }

    /// Reject nil identities, out-of-range widths or coordinates, and zero-length segments.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.id.is_nil()
            || self.layer.is_nil()
            || !(MIN_WIDTH_NM..=MAX_WIDTH_NM).contains(&self.width_nm)
            || !crate::wire::valid_point(&self.start)
            || !crate::wire::valid_point(&self.end)
            || self.start == self.end
        {
            return Err(format!(
                "a track needs a non-nil identity and layer, a width from {MIN_WIDTH_NM} to {MAX_WIDTH_NM} nm and two distinct bounded ends"
            ));
        }
        Ok(())
    }
}

/// A plated through hole whose round pad spans every copper layer of the board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Via {
    pub id: Uuid,
    /// Authored scoped intent; omission follows contacted copper automatically.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::via_nets::stored_assignment")]
    pub net_assignment: Option<crate::nets::NetAddress>,
    /// Lock intent travels with this via through native save and clipboard copies.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    pub at: Point,
    pub diameter_nm: i64,
    pub drill_nm: i64,
}

impl Via {
    /// Inspection always exposes lock state, including the default false omitted from native files.
    pub fn inspect(&self) -> Value {
        let mut value = json!(self);
        value["locked"] = json!(self.locked);
        value
    }

    /// Reject nil identities, out-of-range positions, and drills that do not fit inside the pad.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.id.is_nil()
            || self.net_assignment.is_some_and(|a| a.project.is_nil() || a.scope.id.is_nil() || a.net.is_nil())
            || !crate::wire::valid_point(&self.at)
            || !(MIN_VIA_DRILL_NM..self.diameter_nm).contains(&self.drill_nm)
            || self.diameter_nm > MAX_VIA_DIAMETER_NM
        {
            return Err(format!(
                "a via needs a non-nil identity, a bounded position, a diameter up to {MAX_VIA_DIAMETER_NM} nm and a drill from {MIN_VIA_DRILL_NM} nm up to but smaller than its diameter"
            ));
        }
        Ok(())
    }
}

/// Refuse an over-capacity board before inspecting records or deriving geometry.
fn validate_counts(tracks: usize, vias: usize) -> Result<(), String> {
    if tracks > MAX_TRACKS {
        return Err(format!("a board can hold at most {MAX_TRACKS} track segments"));
    }
    if vias > MAX_VIAS {
        return Err(format!("a board can hold at most {MAX_VIAS} vias"));
    }
    Ok(())
}

/// Copper-layer identities from the current board setup; empty on an unconfigured board.
fn copper_layer_ids(project: &Project) -> BTreeSet<Uuid> {
    project.board.setup.iter().flat_map(|setup| setup.layers()).filter(|layer| layer.purpose == LayerPurpose::Copper).map(|layer| layer.id).collect()
}

/// Validate every segment and via of a candidate board, then sort each by ID for diff-stable
/// saves and bulk command lookups. Vias need at least two copper layers to pass between.
pub(crate) fn validate(project: &Project, tracks: &mut [Track], vias: &mut [Via]) -> Result<(), String> {
    validate_counts(tracks.len(), vias.len())?;
    let copper = copper_layer_ids(project);
    let mut ids = BTreeSet::new();
    for track in tracks.iter() {
        check_track(track, &copper, &mut ids)?;
    }
    check_via_layers(!vias.is_empty(), &copper)?;
    for via in vias.iter() {
        check_via(via, &mut ids)?;
    }
    tracks.sort_by_key(|track| track.id);
    vias.sort_by_key(|via| via.id);
    Ok(())
}

/// Check one segment of a candidate in `validate` order, recording its identity.
fn check_track(track: &Track, copper: &BTreeSet<Uuid>, ids: &mut BTreeSet<Uuid>) -> Result<(), String> {
    track.validate()?;
    if !ids.insert(track.id) {
        return Err("duplicate track identity".into());
    }
    if !copper.contains(&track.layer) {
        return Err("tracks must stay on a copper layer of the board setup".into());
    }
    Ok(())
}

/// Vias pass between copper layers, so a candidate with any needs at least two.
fn check_via_layers(any_vias: bool, copper: &BTreeSet<Uuid>) -> Result<(), String> {
    if any_vias && copper.len() < 2 {
        return Err("vias need at least two copper layers in the board setup".into());
    }
    Ok(())
}

/// Check one via of a candidate after every segment, recording its identity.
fn check_via(via: &Via, ids: &mut BTreeSet<Uuid>) -> Result<(), String> {
    via.validate()?;
    if !ids.insert(via.id) {
        return Err("duplicate track or via identity".into());
    }
    Ok(())
}

/// Copper islands for one board snapshot: which pads, tracks and vias are joined by copper.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Copper {
    /// Island index for each physical pad, keyed by footprint ID and pad number.
    pub pads: BTreeMap<(String, String), usize>,
    /// Island index for each track segment.
    pub tracks: BTreeMap<Uuid, usize>,
    /// Island index for each via.
    pub vias: BTreeMap<Uuid, usize>,
}

/// Resolved pad geometry and net membership used by connectivity, DRC and footprint edits.
#[derive(Clone)]
pub(super) struct Land {
    pub(super) key: (String, String),
    pub(super) corners: [Point; 4],
    pub(super) layers: Vec<Uuid>,
    pub(super) net: Option<String>,
    center: Point,
    shape: PadShape,
    corner_radius_nm: i64,
    size_nm: (i64, i64),
}

impl Land {
    /// Derive the copper contour from resolved corners and pad shape in board nanometers.
    pub(super) fn contour(&self) -> PadContour {
        PadContour::pad(self.center, self.corners, self.shape, self.corner_radius_nm, self.size_nm)
    }
}

impl<'a> BoardView<'a> {
    /// Authored track segments in stable ID order.
    pub fn tracks(self) -> &'a [Track] {
        &self.project().board.tracks
    }

    /// Authored vias in stable ID order.
    pub fn vias(self) -> &'a [Via] {
        &self.project().board.vias
    }

    /// Group physical pads, tracks and vias into islands joined by overlapping copper: tracks on
    /// a shared layer, vias on any layer. Legacy center-only markers and unconfigured pads have
    /// no copper and stay isolated. Circular copper returns an unsupported-geometry error.
    pub fn copper(self) -> Result<Copper, String> {
        self.require_linear_copper()?;
        Ok(islands(&lands(self), self.tracks(), self.vias()).0)
    }

    /// Where routed copper would go if footprint `id` took the given pose: the tracks whose ends
    /// ride on its pads, with those ends moved as `pcb.set_footprint_transform` would move them.
    /// For previews only; nothing is edited, and an unknown ID carries nothing. Circular
    /// copper returns an error rather than an incomplete straight-only preview.
    pub fn carried_tracks(self, id: &str, position: Point, rotation: u16, bottom: bool) -> Result<Vec<Track>, String> {
        self.require_linear_copper()?;
        let Some(number) = crate::schematic::wiring::parse_id(id, "footprint-").filter(|_| !self.tracks().is_empty()) else {
            return Ok(Vec::new());
        };
        let mut next = self.project().clone();
        let board = std::sync::Arc::make_mut(&mut next.board);
        let Ok(index) = board.footprints.binary_search_by_key(&number, |f| f.id) else { return Ok(Vec::new()) };
        let footprint = &mut board.footprints[index];
        (footprint.x_nm, footprint.y_nm, footprint.rotation, footprint.bottom) = (position.x_nm, position.y_nm, rotation, bottom);
        let Some(after) = next.board_view(self.target) else { return Ok(Vec::new()) };
        Ok(carry(self, after, &BTreeSet::from([id.to_owned()])))
    }

    /// Resolve one unambiguous intended net per copper island, including explicit isolated vias.
    /// Conflicting contacts expose no arbitrary display net; physical islands remain separate.
    /// Circular copper returns an error rather than incomplete membership.
    pub fn copper_nets(self) -> Result<BTreeMap<Uuid, String>, String> {
        self.copper_with_nets().map(|(_, nets)| nets)
    }

    /// Resolve physical islands and complete authored/contact-derived intent in one contact pass.
    /// Circular copper returns an error before any partial geometry is resolved.
    pub fn copper_with_nets(self) -> Result<(Copper, BTreeMap<Uuid, String>), String> {
        self.require_linear_copper()?;
        let lands = lands(self);
        let (tracks, vias) = (self.tracks(), self.vias());
        let (copper, members) = islands(&lands, tracks, vias);
        let members = if vias.iter().any(|v| v.net_assignment.is_some()) { with_pad_contacts(&lands, &copper, members) } else { members };
        let intent = self.project().board_intent_memberships();
        let intrinsic = super::via_nets::node_membership(self.project(), &lands, tracks.len(), vias, &intent);
        let island_net: Vec<_> = members
            .iter()
            .map(|nodes| {
                let unresolved = nodes
                    .iter()
                    .filter_map(|n| n.checked_sub(lands.len() + tracks.len()).and_then(|i| vias.get(i)))
                    .any(|v| v.net_assignment.is_some_and(|a| self.project().via_net_state(a) != "active"));
                let ids: BTreeSet<_> = nodes.iter().filter_map(|n| intrinsic[*n]).collect();
                if unresolved || ids.len() != 1 {
                    return None;
                }
                intent.names.get(ids.first()?).cloned().or_else(|| nodes.iter().find_map(|n| lands.get(*n)?.net.clone()))
            })
            .collect();
        let mut nets = BTreeMap::new();
        for (nodes, name) in members.iter().zip(island_net) {
            let Some(name) = name else { continue };
            for node in nodes {
                if let Some(index) = node.checked_sub(lands.len()) {
                    let id = tracks.get(index).map(|t| t.id).or_else(|| index.checked_sub(tracks.len()).and_then(|i| vias.get(i)).map(|v| v.id));
                    if let Some(id) = id {
                        nets.insert(id, name.clone());
                    }
                }
            }
        }
        Ok((copper, nets))
    }
}

/// Resolve each physical pad's land, layers and net once per snapshot. Markers without
/// physical copper and pads on an unconfigured board are omitted.
pub(super) fn lands(view: BoardView<'_>) -> Vec<Land> {
    lands_from_footprints(view.footprints())
}

/// Reuse already projected footprints, including bounded checker preparation batches.
pub(super) fn lands_from_footprints<'a>(footprints: impl Iterator<Item = super::view::FootprintView<'a>>) -> Vec<Land> {
    footprints
        .flat_map(|footprint| {
            let id = footprint.id.clone();
            footprint.pads.into_iter().filter_map(move |pad| {
                let corners = pad.copper?;
                let Some(CopperLayers::Resolved { layers, .. }) = pad.copper_layers else { return None };
                Some(Land {
                    key: (id.clone(), pad.number),
                    corners,
                    layers,
                    net: pad.net,
                    center: pad.center,
                    shape: pad.shape,
                    corner_radius_nm: pad.corner_radius_nm,
                    size_nm: pad.size_nm,
                })
            })
        })
        .collect()
}

/// Union-find over lands, then tracks, then vias. Returns islands plus each island's member
/// node indices, numbered in that order.
pub(super) fn islands(lands: &[Land], tracks: &[Track], vias: &[Via]) -> (Copper, Vec<Vec<usize>>) {
    build_islands(lands, tracks, vias, false)
}

/// Join direct pad contacts onto existing islands without repeating track/via contact searches.
/// The original island map remains available for the separate clearance check. Returned members
/// retain canonical node ordering, independently of the pad sweep's contact order.
pub(super) fn with_pad_contacts(lands: &[Land], copper: &Copper, members: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    let contours: Vec<_> = lands.iter().map(Land::contour).collect();
    let bounds: Vec<_> = contours.iter().map(PadContour::bounds).collect();
    let mut parent: Vec<usize> = (0..members.len()).collect();
    for_each_pad_contact(lands, &contours, &bounds, |i, j| join(&mut parent, copper.pads[&lands[i].key], copper.pads[&lands[j].key]));
    let mut merged = BTreeMap::<usize, Vec<usize>>::new();
    for (island, nodes) in members.into_iter().enumerate() {
        merged.entry(find(&mut parent, island)).or_default().extend(nodes);
    }
    merged
        .into_values()
        .map(|mut nodes| {
            nodes.sort_unstable();
            nodes
        })
        .collect()
}

/// Find an island's canonical root, compressing its path in the local union-find table.
fn find(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// Merge two local contact groups, retaining the lower root for deterministic island ordering.
fn join(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (find(parent, a), find(parent, b));
    if a != b {
        parent[a.max(b)] = a.min(b);
    }
}

/// Build islands from copper contours. Footprint edit validation also joins direct pad pairs.
fn build_islands(lands: &[Land], tracks: &[Track], vias: &[Via], join_pad_pairs: bool) -> (Copper, Vec<Vec<usize>>) {
    let mut builder = IslandBuilder::new(lands, tracks, vias);
    while !builder.step(usize::MAX) {}
    if join_pad_pairs {
        for_each_pad_contact(lands, &builder.contours[..lands.len()], &builder.bounds[..lands.len()], |i, j| join(&mut builder.parent, i, j));
    }
    builder.finish(lands, tracks, vias)
}

/// Copper items (pads, tracks and vias together) below which island contact phases enumerate
/// neighbors directly instead of building a bounds index. Retains the original small-board
/// threshold for contact preparation.
pub(super) const PAIRWISE_BELOW: usize = 128;

/// Whether island contact phases enumerate lower-numbered neighbors directly for `items`
/// copper items instead of using a bounds index. Both paths apply the same layer and contact tests.
fn compares_every_pair(items: usize) -> bool {
    items < PAIRWISE_BELOW
}

/// Resumable pair enumeration for the existing copper-island predicates. Small boards enumerate
/// lower-numbered neighbors directly; larger ones traverse a four-axis bounds tree. A step
/// charges every visited branch and leaf, not just emitted pairs. Overlap in all projections
/// can still require visiting every eligible pair.
/// Pad-to-pad contacts stay separate from legacy clearance islands.
pub(super) struct IslandBuilder {
    parent: Vec<usize>,
    contours: Vec<PadContour>,
    bounds: Vec<[f64; 4]>,
    layers: Vec<Vec<Uuid>>,
    /// None when lower-numbered neighbors are enumerated directly.
    index: Option<PairIndex>,
    search: Search,
    /// The node being joined to its lower-numbered neighbors.
    node: usize,
    /// That node's remaining neighbors to test.
    nearby: Nearby,
}

/// The lower-numbered nodes still to test against the current node.
enum Nearby {
    /// Not looked up yet.
    Lookup,
    /// An active search. When enumerating directly, this is the exclusive upper bound of
    /// the remaining neighbors. Indexed builders resume their separate traversal cursor.
    Active(usize),
}

impl IslandBuilder {
    /// Capture bounded primitive contours once, without evaluating any contact pairs.
    pub(super) fn new(lands: &[Land], tracks: &[Track], vias: &[Via]) -> Self {
        let contours: Vec<_> = lands
            .iter()
            .map(Land::contour)
            .chain(tracks.iter().map(|t| PadContour::capsule(t.start, t.end, t.width_nm as f64 / 2.)))
            .chain(vias.iter().map(|v| PadContour::capsule(v.at, v.at, v.diameter_nm as f64 / 2.)))
            .collect();
        let bounds: Vec<_> = contours.iter().map(PadContour::bounds).collect();
        let layers: Vec<_> = lands.iter().map(|l| l.layers.clone()).chain(tracks.iter().map(|t| vec![t.layer])).collect();
        let pairwise = compares_every_pair(contours.len());
        Self::with_path(lands.len(), contours, bounds, layers, pairwise)
    }

    /// Build on the chosen path; tests compare both paths on the same board.
    fn with_path(lands: usize, contours: Vec<PadContour>, bounds: Vec<[f64; 4]>, layers: Vec<Vec<Uuid>>, pairwise: bool) -> Self {
        let index = (!pairwise).then(|| PairIndex::new(contours.iter()));
        Self {
            parent: (0..contours.len()).collect(),
            contours,
            bounds,
            layers,
            index,
            search: Search::default(),
            node: lands,
            nearby: Nearby::Lookup,
        }
    }

    /// The same builder forced onto one path, to check that both give the same islands.
    #[cfg(test)]
    pub(super) fn on_path(self, pairwise: bool) -> Self {
        let node = self.node;
        let mut builder = Self::with_path(0, self.contours, self.bounds, self.layers, pairwise);
        (builder.parent, builder.node) = (self.parent, node);
        builder
    }

    /// Resume direct pad contacts from legacy roots, preserving the separate clearance map.
    pub(super) fn pad_contacts(lands: &[Land], tracks: &[Track], vias: &[Via], members: &[Vec<usize>]) -> Self {
        let mut builder = Self::new(lands, tracks, vias);
        for nodes in members {
            if let Some(first) = nodes.first() {
                for &node in nodes {
                    builder.parent[node] = *first;
                }
            }
        }
        builder.node = 0;
        builder.nearby = Nearby::Lookup;
        builder
    }

    /// Evaluate at most `budget` work units of direct pad pairs, without rechecking track/via
    /// contacts. Only pads, the first `pads` nodes, are joined.
    pub(super) fn step_pads(&mut self, pads: usize, budget: usize) -> bool {
        self.advance(pads, budget)
    }

    /// Evaluate at most `budget` work units. Vias span the configured copper stack, as in
    /// ordinary connectivity. A zero budget does no work and never claims completion early.
    pub(super) fn step(&mut self, budget: usize) -> bool {
        self.advance(self.parent.len(), budget)
    }

    /// Join each remaining node before `end` to every lower-numbered node whose copper touches it on a
    /// shared layer. Each pair is decided once, from its higher node, by the same predicate
    /// whatever the index returns, so the islands do not depend on the lookup strategy.
    fn advance(&mut self, end: usize, budget: usize) -> bool {
        for _ in 0..budget {
            let i = self.node;
            if i >= end {
                return true;
            }
            let next = match (&mut self.nearby, &self.index) {
                (Nearby::Lookup, index) => {
                    if let Some(index) = index {
                        index.begin(&mut self.search, 0.);
                    }
                    self.nearby = Nearby::Active(i);
                    continue;
                }
                (Nearby::Active(below), None) => below.checked_sub(1).inspect(|&j| *below = j),
                (Nearby::Active(_), Some(index)) => match index.next(i, &mut self.search) {
                    Hit::Pending => continue,
                    Hit::Candidate(candidate) => Some(candidate),
                    Hit::Done => None,
                },
            };
            let Some(j) = next else {
                self.node += 1;
                self.nearby = Nearby::Lookup;
                continue;
            };
            if self.routed_contact(i, j) {
                join(&mut self.parent, i, j);
            }
        }
        self.node >= end
    }

    /// Test one routed pair in node order (`i > j`), with any via first. Pad-only pairs
    /// are checked separately by callers that need direct land overlap.
    fn routed_contact(&self, i: usize, j: usize) -> bool {
        routed_contact(&self.layers, &self.bounds, &self.contours, i, j)
    }

    /// Canonicalize completed roots to the existing pad/track/via identity maps.
    pub(super) fn finish(mut self, lands: &[Land], tracks: &[Track], vias: &[Via]) -> (Copper, Vec<Vec<usize>>) {
        let parent = &mut self.parent;
        let mut roots = BTreeMap::new();
        let mut members: Vec<Vec<usize>> = Vec::new();
        let mut copper = Copper::default();
        for node in 0..parent.len() {
            let root = find(parent, node);
            let next = roots.len();
            let island = *roots.entry(root).or_insert(next);
            if island == members.len() {
                members.push(Vec::new());
            }
            members[island].push(node);
            if let Some(land) = lands.get(node) {
                copper.pads.insert(land.key.clone(), island);
            } else if let Some(track) = tracks.get(node - lands.len()) {
                copper.tracks.insert(track.id, island);
            } else {
                copper.vias.insert(vias[node - lands.len() - tracks.len()].id, island);
            }
        }
        (copper, members)
    }
}

/// Test a routed pair in descending node order. Via nodes follow all explicitly layered nodes.
fn routed_contact(layers: &[Vec<Uuid>], bounds: &[[f64; 4]], contours: &[PadContour], i: usize, j: usize) -> bool {
    let shares_layer = match (layers.get(i), layers.get(j)) {
        (Some(a), Some(b)) => a.iter().any(|layer| b.contains(layer)),
        (None, Some(b)) => !b.is_empty(),
        _ => true,
    };
    let (a, b) = (bounds[i], bounds[j]);
    shares_layer && a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3] && contours[i].touches(&contours[j])
}

/// Sweep overlapping pad bounds and visit each pair whose copper touches on a shared layer.
fn for_each_pad_contact(lands: &[Land], contours: &[PadContour], bounds: &[[f64; 4]], mut visit: impl FnMut(usize, usize)) {
    let mut order: Vec<_> = (0..lands.len()).collect();
    order.sort_by(|&a, &b| bounds[a][0].total_cmp(&bounds[b][0]));
    for (position, &i) in order.iter().enumerate() {
        let [_, top, right, bottom] = bounds[i];
        for &j in &order[position + 1..] {
            let [left, other_top, _, other_bottom] = bounds[j];
            if left > right {
                break;
            }
            if top <= other_bottom
                && other_top <= bottom
                && lands[i].layers.iter().any(|layer| lands[j].layers.contains(layer))
                && contours[i].overlaps(&contours[j])
            {
                visit(i, j);
            }
        }
    }
}

/// Name the first two distinct nets joined by an island that contains one of the `added`
/// tracks or vias, if any. Islands without new copper are ignored so pre-existing shorts
/// do not block unrelated routes.
#[cfg(test)]
fn short(project: &Project, added: &BTreeSet<Uuid>) -> Option<(String, String)> {
    short_for_copper_edit(project, added, &BTreeSet::new())
}

/// Check the islands touching edited tracks, vias or footprint lands for shorts.
/// Footprint edits use pad contours and include direct pad contacts on a shared layer.
/// Unrelated pre-existing islands remain outside the check.
pub(crate) fn short_for_copper_edit(project: &Project, added: &BTreeSet<Uuid>, footprints: &BTreeSet<String>) -> Option<(String, String)> {
    let view = project.board_view(project.board_target()?)?;
    let lands = lands(view);
    let (tracks, vias) = (view.tracks(), view.vias());
    let (_, members) = build_islands(&lands, tracks, vias, !footprints.is_empty() || vias.iter().any(|v| v.net_assignment.is_some()));
    short_from_members(project, &lands, tracks, vias, &members, added, footprints)
}

/// Apply the existing short policy to completed contact groups, without recomputing islands.
fn short_from_members(
    project: &Project,
    lands: &[Land],
    tracks: &[Track],
    vias: &[Via],
    members: &[Vec<usize>],
    added: &BTreeSet<Uuid>,
    footprints: &BTreeSet<String>,
) -> Option<(String, String)> {
    let is_added = |node: usize| {
        if let Some(land) = lands.get(node) {
            return footprints.contains(&land.key.0);
        }
        let index = node.checked_sub(lands.len());
        let track = index.and_then(|index| tracks.get(index)).map(|track| track.id);
        let via = index.and_then(|index| index.checked_sub(tracks.len())).and_then(|index| vias.get(index)).map(|via| via.id);
        track.or(via).is_some_and(|id| added.contains(&id))
    };
    let intent = project.board_intent_memberships();
    let intrinsic = super::via_nets::node_membership(project, lands, tracks.len(), vias, &intent);
    members.iter().filter(|island| island.iter().any(|node| is_added(*node))).find_map(|island| {
        let mut nets = island.iter().filter_map(|node| intrinsic[*node]);
        let first = nets.next()?;
        let second = nets.find(|net| *net != first)?;
        let name = |id| {
            intent
                .names
                .get(&id)
                .cloned()
                .or_else(|| island.iter().find_map(|n| (intrinsic[*n] == Some(id)).then(|| lands.get(*n)?.net.clone()).flatten()))
                .unwrap_or_else(|| format!("net {}", id + 1))
        };
        Some((name(first), name(second)))
    })
}

/// Map `point` from one placement of a pad land to another, keeping its place relative to the
/// land. The land's first corner and its two edges from there form the frame, so translation,
/// rotation and mirroring all carry over.
fn reframe(point: Point, from: &[Point; 4], to: &[Point; 4]) -> Point {
    let axes = |c: &[Point; 4]| {
        let o = (c[0].x_nm as f64, c[0].y_nm as f64);
        (o, (c[1].x_nm as f64 - o.0, c[1].y_nm as f64 - o.1), (c[3].x_nm as f64 - o.0, c[3].y_nm as f64 - o.1))
    };
    let ((ox, oy), (ux, uy), (vx, vy)) = axes(from);
    let det = ux * vy - uy * vx;
    if det == 0. {
        return point;
    }
    let (dx, dy) = (point.x_nm as f64 - ox, point.y_nm as f64 - oy);
    let (u, v) = ((dx * vy - dy * vx) / det, (ux * dy - uy * dx) / det);
    let ((ox, oy), (ux, uy), (vx, vy)) = axes(to);
    Point { x_nm: (ox + u * ux + v * vx).round() as i64, y_nm: (oy + u * uy + v * vy).round() as i64 }
}

/// Tracks whose ends ride on pads of the `moved` footprints, by footprint ID, between `before`
/// and `after` (the same board before and after those footprints moved). Each end that
/// overlapped one of their pads on the track's layer keeps its place on that pad; the rest of
/// the track stretches. An end whose pad no longer reaches the track's layer, such as an SMD
/// pad flipped to the other side, stays where it was. Only tracks that change are returned.
pub(super) fn carry(before: BoardView<'_>, after: BoardView<'_>, moved: &BTreeSet<String>) -> Vec<Track> {
    let old = carrying_lands(before, moved);
    if old.is_empty() {
        return Vec::new();
    }
    let new: BTreeMap<(String, String), Land> =
        lands(after).into_iter().filter(|land| moved.contains(&land.key.0)).map(|land| (land.key.clone(), land)).collect();
    let follow = |end: Point, track: &Track| {
        let Some(land) = carrier(&old, track, end) else { return end };
        match new.get(&land.key) {
            Some(moved) if moved.layers.contains(&track.layer) => reframe(end, &land.corners, &moved.corners),
            _ => end,
        }
    };
    before
        .tracks()
        .iter()
        .filter_map(|track| {
            let (start, end) = (follow(track.start, track), follow(track.end, track));
            ((start, end) != (track.start, track.end)).then(|| Track { start, end, ..track.clone() })
        })
        .collect()
}

/// Capture moved pad contours once for contact-preserving footprint/group translation.
fn carrying_lands(view: BoardView<'_>, moved: &BTreeSet<String>) -> Vec<(Land, PadContour)> {
    lands(view)
        .into_iter()
        .filter(|land| moved.contains(&land.key.0))
        .map(|land| {
            let contour = land.contour();
            (land, contour)
        })
        .collect()
}

/// Use the same physical pad overlap policy for individual poses and group translation.
fn carrier<'a>(old: &'a [(Land, PadContour)], track: &Track, end: Point) -> Option<&'a Land> {
    old.iter()
        .find(|(land, contour)| land.layers.contains(&track.layer) && contour.overlaps_segment(end, end, track.width_nm as f64 / 2.))
        .map(|(land, _)| land)
}

/// Capture which unselected track ends follow selected pads, outside pointer-motion frames.
pub(super) fn carried_ends(view: BoardView<'_>, moved: &BTreeSet<String>) -> BTreeMap<Uuid, (bool, bool)> {
    let old = carrying_lands(view, moved);
    view.tracks()
        .iter()
        .filter_map(|track| {
            let ends = (carrier(&old, track, track.start).is_some(), carrier(&old, track, track.end).is_some());
            (ends.0 || ends.1).then_some((track.id, ends))
        })
        .collect()
}

/// Bring routed copper along with the `moved` footprints (by footprint ID) of `next`, which
/// differs from `before` only in their placements; see [`carry`]. Refuses the edit when a
/// carried track would collapse or leave the coordinate range, or when the moved pads or the
/// carried tracks would join two nets.
pub(super) fn carry_copper(before: &Project, next: &mut Project, moved: &BTreeSet<String>) -> Result<(), CommandError> {
    let Some(target) = before.board_target() else { return Ok(()) };
    let (Some(old), Some(new)) = (before.board_view(target), next.board_view(target)) else { return Ok(()) };
    let carried = carry(old, new, moved);
    for track in &carried {
        track.validate().map_err(|_| CommandError::InvalidParams("this move would collapse or overextend an attached track".into()))?;
    }
    let added: BTreeSet<Uuid> = carried.iter().map(|track| track.id).collect();
    if !carried.is_empty() {
        let tracks = &mut std::sync::Arc::make_mut(&mut next.board).tracks;
        for track in carried {
            if let Ok(index) = tracks.binary_search_by_key(&track.id, |t| t.id) {
                tracks[index] = track;
            }
        }
    }
    if let Some((a, b)) = short_for_copper_edit(next, &added, moved) {
        return Err(CommandError::InvalidParams(format!("this move would connect nets {a} and {b}")));
    }
    Ok(())
}

/// Parse a JSON value into a typed parameter, reporting serde's message as invalid input.
fn param<T: serde::de::DeserializeOwned>(params: &Value, name: &str) -> Result<T, CommandError> {
    serde_json::from_value(params[name].clone()).map_err(|error| CommandError::InvalidParams(format!("{name}: {error}")))
}

/// A route's switch to another copper layer through a via at one of its interior vertices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerChange {
    /// Index into the route's `points` of the vertex where the via sits, after the first.
    pub at: usize,
    /// The copper layer the route continues on after the via.
    pub layer: Uuid,
}

/// Parse an optional list of unique existing IDs; absent means none.
fn ids(params: &Value, name: &str, existing: impl Fn(&Uuid) -> bool) -> Result<BTreeSet<Uuid>, CommandError> {
    if params.get(name).is_none() {
        return Ok(BTreeSet::new());
    }
    let ids: Vec<Uuid> = param(params, name)?;
    let unique: BTreeSet<_> = ids.iter().copied().collect();
    if unique.len() != ids.len() || !ids.iter().all(existing) {
        return Err(CommandError::InvalidParams(format!("{name} must be unique IDs of existing board items")));
    }
    Ok(unique)
}

impl Session {
    /// Add one routed path, place one standalone via, move, reshape or resize addressed tracks and
    /// vias, or delete them, atomically. Reshaping sets explicit ends and refuses a segment whose
    /// ends would meet. A standalone via may not sit exactly on another via. Moved
    /// and resized items count as new copper for the short check, so an edit that makes them
    /// touch another net's copper is refused whole. A no-op edit leaves history unchanged. A path of N distinct
    /// consecutive points becomes N-1 segments with one width, starting on `layer`. Each layer
    /// change places one via at a vertex after the first, in strictly increasing order, and later
    /// segments continue on its layer; a change at the last vertex ends the route on a via. The whole candidate is validated first: unknown layers or IDs,
    /// invalid geometry or via sizes, and copper that would join two different nets leave the
    /// board, revision and undo history unchanged. Omitted creation dimensions use the shared
    /// board preferences; explicit dimensions remain authored geometry checked by DRC.
    pub(crate) fn edit_tracks(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        let mut job = self.prepare_copper_edit(command, params)?;
        while !job.step(usize::MAX) {}
        let prepared = job.take_result()?.ok_or_else(|| CommandError::Unavailable("copper preparation returned no result".into()))?;
        self.commit_copper_edit(prepared).map(drop)
    }
}

/// Count-checked route input, still without sizes or allocated board item identities.
struct RouteAddition {
    layer: Uuid,
    points: Vec<Point>,
    changes: Vec<LayerChange>,
}

/// Parse a route's ordered points and layer changes, counting even a final-vertex via
/// before defaults or source geometry are queried.
fn route_addition(project: &Project, params: &Value) -> Result<RouteAddition, CommandError> {
    let layer = param(params, "layer")?;
    let points: Vec<Point> = param(params, "points")?;
    if !(2..=MAX_PATH_POINTS).contains(&points.len()) || points.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(CommandError::InvalidParams(format!("a route needs 2 to {MAX_PATH_POINTS} points with distinct neighbors")));
    }
    let changes: Vec<LayerChange> = if params.get("layer_changes").is_some() { param(params, "layer_changes")? } else { Vec::new() };
    let copper = copper_layer_ids(project);
    if !copper.contains(&layer) {
        return Err(CommandError::InvalidParams("tracks must stay on a copper layer of the board setup".into()));
    }
    let (mut previous, mut current) = (0, layer);
    for change in &changes {
        if change.at <= previous || change.at >= points.len() {
            return Err(CommandError::InvalidParams("layer changes need strictly increasing vertex indices after the first".into()));
        }
        if !copper.contains(&change.layer) {
            return Err(CommandError::InvalidParams("a layer change must move to a copper layer of the board setup".into()));
        }
        if change.layer == current {
            return Err(CommandError::InvalidParams("a layer change must move to a different copper layer".into()));
        }
        (previous, current) = (change.at, change.layer);
    }
    validate_counts(project.board.tracks.len() + points.len() - 1, project.board.vias.len() + changes.len()).map_err(CommandError::InvalidParams)?;
    Ok(RouteAddition { layer, points, changes })
}

/// Edit a private candidate without publishing. Refuse inevitable count overflow before
/// resolving source-contact defaults (which may build connectivity) or allocating identities.
/// Preparation validates final membership and explicit hard limits before publication.
fn edit_candidate(project: &Project, command: &str, params: &Value) -> Result<(Option<Project>, BTreeSet<Uuid>), CommandError> {
    let route = (command == ADD_TRACKS).then(|| route_addition(project, params)).transpose()?;
    let via_at = if command == ADD_VIA {
        let at: Point = param(params, "at")?;
        if !crate::wire::valid_point(&at) {
            return Err(CommandError::InvalidParams("the via position is out of range".into()));
        }
        validate_counts(project.board.tracks.len(), project.board.vias.len() + 1).map_err(CommandError::InvalidParams)?;
        if project.board.vias.iter().any(|via| via.at == at) {
            return Err(CommandError::InvalidParams("a via already sits at this position".into()));
        }
        Some(at)
    } else {
        None
    };
    let promoted = if command == ADD_VIA && params.get("target").is_some() { Some(project.with_via_target(&params["target"])?) } else { None };
    let original = project;
    let project = promoted.as_ref().map_or(project, |(p, _)| p);
    let effective = project.board.rules.as_ref().map_or_else(super::rules::Values::default, |rules| rules.values);
    let needs_defaults = match command {
        ADD_TRACKS => {
            params.get("width_nm").is_none()
                || (params.get("layer_changes").and_then(Value::as_array).is_some_and(|changes| !changes.is_empty())
                    && (params.get("via_diameter_nm").is_none() || params.get("via_drill_nm").is_none()))
        }
        ADD_VIA => params.get("via_diameter_nm").is_none() || params.get("via_drill_nm").is_none(),
        _ => false,
    };
    // Explicit GUI/pointer dimensions need no source-geometry lookup. The completed candidate
    // still validates their actual classified membership before publication.
    let policy = if needs_defaults {
        let view = project
            .board_view(project.resolve_board(params.get("board").and_then(Value::as_str))?)
            .ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
        let (at, layer) = if let Some(route) = &route {
            (route.points[0], Some(route.layer))
        } else {
            (via_at.ok_or_else(|| CommandError::InvalidParams("A via needs a position.".into()))?, None)
        };
        if let Some((_, address)) = &promoted {
            let resolved = project.resolve_net_assignment(*address)?;
            Some(super::class_rules::Applied { nets: vec![*address], bindings: resolved.bindings, resolution: resolved.resolution })
                .filter(|p| p.has_dimensions())
        } else {
            view.route_rules_at(at, layer)?.filter(|policy| policy.has_dimensions())
        }
    } else {
        None
    };
    let mut tracks = project.board.tracks.clone();
    let mut vias = project.board.vias.clone();
    let mut added = BTreeSet::new();
    if let Some(RouteAddition { mut layer, points, changes }) = route {
        let explicit_width = params.get("width_nm").map(|_| param(params, "width_nm")).transpose()?;
        let width_nm = match &policy {
            Some(policy) => policy.track_width(explicit_width)?,
            None => explicit_width.unwrap_or(effective.preferred_track_width_nm),
        };
        let size = |name| -> Result<i64, CommandError> {
            if params.get(name).is_none() {
                Ok(match name {
                    "via_diameter_nm" => effective.preferred_via_diameter_nm,
                    _ => effective.preferred_via_drill_nm,
                })
            } else {
                param(params, name)
            }
        };
        let (diameter_nm, drill_nm) = if let Some(policy) = &policy
            && !changes.is_empty()
        {
            policy.via_sizes(
                params.get("via_diameter_nm").map(|_| param(params, "via_diameter_nm")).transpose()?,
                params.get("via_drill_nm").map(|_| param(params, "via_drill_nm")).transpose()?,
            )?
        } else {
            (size("via_diameter_nm")?, size("via_drill_nm")?)
        };
        let mut changes = changes.iter().peekable();
        for (index, pair) in points.windows(2).enumerate() {
            if let Some(change) = changes.next_if(|change| change.at == index) {
                layer = change.layer;
                let id = crate::fresh_id()?;
                added.insert(id);
                vias.push(Via { id, net_assignment: None, locked: false, at: pair[0], diameter_nm, drill_nm });
            }
            let id = crate::fresh_id()?;
            added.insert(id);
            tracks.push(Track { id, locked: false, layer, width_nm, start: pair[0], end: pair[1] });
        }
        // A change at the final vertex ends the route on a via, ready to continue on that layer.
        if let Some(change) = changes.next() {
            let id = crate::fresh_id()?;
            added.insert(id);
            vias.push(Via { id, net_assignment: None, locked: false, at: points[change.at], diameter_nm, drill_nm });
        }
    } else if let Some(at) = via_at {
        let id = crate::fresh_id()?;
        added.insert(id);
        let explicit_diameter = params.get("via_diameter_nm").map(|_| param(params, "via_diameter_nm")).transpose()?;
        let explicit_drill = params.get("via_drill_nm").map(|_| param(params, "via_drill_nm")).transpose()?;
        let (diameter_nm, drill_nm) = match &policy {
            Some(policy) => policy.via_sizes(explicit_diameter, explicit_drill)?,
            None => (explicit_diameter.unwrap_or(effective.preferred_via_diameter_nm), explicit_drill.unwrap_or(effective.preferred_via_drill_nm)),
        };
        vias.push(Via { id, net_assignment: promoted.as_ref().map(|(_, a)| *a), locked: false, at, diameter_nm, drill_nm });
    } else if command == super::via_nets::SET_VIA_NET {
        let changed = ids(params, "vias", |id| vias.binary_search_by_key(id, |v| v.id).is_ok())?;
        if changed.is_empty()
            || (params.get("target").is_some() && params.get("automatic").is_some())
            || params.get("target").is_some() == (params.get("automatic") == Some(&json!(true)))
        {
            return Err(CommandError::InvalidParams("Choose a target or automatic:true for existing vias.".into()));
        }
        let (next, assignment) = if let Some(target) = params.get("target") {
            let (next, address) = project.with_via_target(target)?;
            (next, Some(address))
        } else {
            (project.clone(), None)
        };
        let mut assigned = BTreeSet::new();
        for via in vias.iter_mut().filter(|v| changed.contains(&v.id) && v.net_assignment != assignment) {
            via.net_assignment = assignment;
            assigned.insert(via.id);
        }
        if assigned.is_empty() {
            return Ok((None, assigned));
        }
        let mut next = next;
        std::sync::Arc::make_mut(&mut next.board).vias = vias;
        protect_locked(original, &next)?;
        return Ok((Some(next), assigned));
    } else if command == MOVE_TRACKS {
        let moved_tracks = ids(params, "tracks", |id| tracks.binary_search_by_key(id, |track| track.id).is_ok())?;
        let moved_vias = ids(params, "vias", |id| vias.binary_search_by_key(id, |via| via.id).is_ok())?;
        let (dx, dy): (i64, i64) = (param(params, "dx_nm")?, param(params, "dy_nm")?);
        if moved_tracks.is_empty() && moved_vias.is_empty() {
            return Err(CommandError::InvalidParams("name at least one track or via to move".into()));
        }
        let shift = |point: Point| Point { x_nm: point.x_nm.saturating_add(dx), y_nm: point.y_nm.saturating_add(dy) };
        for track in tracks.iter_mut().filter(|track| moved_tracks.contains(&track.id)) {
            (track.start, track.end) = (shift(track.start), shift(track.end));
        }
        for via in vias.iter_mut().filter(|via| moved_vias.contains(&via.id)) {
            via.at = shift(via.at);
        }
        added.extend(moved_tracks.into_iter().chain(moved_vias));
    } else if command == RESHAPE_TRACKS {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Ends {
            track: Uuid,
            start: Point,
            end: Point,
        }
        let ends: Vec<Ends> = param(params, "tracks")?;
        let unique: BTreeSet<_> = ends.iter().map(|ends| ends.track).collect();
        if ends.is_empty() || unique.len() != ends.len() {
            return Err(CommandError::InvalidParams("name each track to reshape once".into()));
        }
        let positions: BTreeMap<Uuid, usize> = tracks.iter().enumerate().map(|(position, track)| (track.id, position)).collect();
        for ends in &ends {
            let &position = positions.get(&ends.track).ok_or_else(|| CommandError::NotFound("unknown track".into()))?;
            (tracks[position].start, tracks[position].end) = (ends.start, ends.end);
        }
        added.extend(unique);
    } else if command == SET_TRACK_WIDTH {
        let changed = ids(params, "tracks", |id| tracks.binary_search_by_key(id, |track| track.id).is_ok())?;
        if changed.is_empty() {
            return Err(CommandError::InvalidParams("name at least one track".into()));
        }
        let width_nm: i64 = param(params, "width_nm")?;
        for track in tracks.iter_mut().filter(|track| changed.contains(&track.id)) {
            track.width_nm = width_nm;
        }
        added.extend(changed);
    } else if command == SET_VIA_SIZE {
        let changed = ids(params, "vias", |id| vias.binary_search_by_key(id, |via| via.id).is_ok())?;
        if changed.is_empty() {
            return Err(CommandError::InvalidParams("name at least one via".into()));
        }
        let (diameter_nm, drill_nm): (i64, i64) = (param(params, "via_diameter_nm")?, param(params, "via_drill_nm")?);
        for via in vias.iter_mut().filter(|via| changed.contains(&via.id)) {
            (via.diameter_nm, via.drill_nm) = (diameter_nm, drill_nm);
        }
        added.extend(changed);
    } else {
        let gone_tracks = ids(params, "tracks", |id| tracks.binary_search_by_key(id, |track| track.id).is_ok())?;
        let gone_vias = ids(params, "vias", |id| vias.binary_search_by_key(id, |via| via.id).is_ok())?;
        if gone_tracks.is_empty() && gone_vias.is_empty() {
            return Err(CommandError::InvalidParams("name at least one track or via to delete".into()));
        }
        tracks.retain(|track| !gone_tracks.contains(&track.id));
        vias.retain(|via| !gone_vias.contains(&via.id));
    }
    validate(project, &mut tracks, &mut vias).map_err(CommandError::InvalidParams)?;
    if tracks == project.board.tracks && vias == project.board.vias {
        return Ok((None, added));
    }
    let mut next = project.clone();
    let board = std::sync::Arc::make_mut(&mut next.board);
    board.tracks = tracks;
    board.vias = vias;
    protect_locked(project, &next)?;
    Ok((Some(next), added))
}

/// Describe a route's layer changes for command clients.
pub(crate) fn layer_changes_schema() -> Value {
    json!({"type":"array","maxItems":MAX_PATH_POINTS - 1,"items":{"type":"object","properties":{
        "at":{"type":"integer","minimum":1,"maximum":MAX_PATH_POINTS - 1},
        "layer":{"type":"string","format":"uuid"}
    },"required":["at","layer"],"additionalProperties":false}})
}

/// Describe a routed path's vertices for command clients.
pub(crate) fn path_schema() -> Value {
    let mut schema = super::drawing::geometry_schema();
    schema["maxItems"] = json!(MAX_PATH_POINTS);
    schema
}

#[cfg(test)]
pub(crate) mod tests;
