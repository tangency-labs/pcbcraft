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
/// Lock or unlock explicitly addressed tracks and vias without changing geometry.
pub const SET_COPPER_LOCKED: &str = "pcb.set_copper_locked";

mod locks;
pub(crate) use locks::protect_locked;
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
/// Segments accepted on one board, bounding saved size and connectivity work.
pub const MAX_TRACKS: usize = 4096;
/// Vias accepted on one board.
pub const MAX_VIAS: usize = 2048;
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

/// Validate every segment and via of a candidate board, then sort each by ID for diff-stable
/// saves. Each track layer must be a copper layer of the board's setup; an unconfigured board
/// has none. Vias need at least two copper layers to pass between.
pub(crate) fn validate(project: &Project, tracks: &mut [Track], vias: &mut [Via]) -> Result<(), String> {
    if tracks.len() > MAX_TRACKS {
        return Err(format!("a board can hold at most {MAX_TRACKS} track segments"));
    }
    if vias.len() > MAX_VIAS {
        return Err(format!("a board can hold at most {MAX_VIAS} vias"));
    }
    let copper: BTreeSet<_> = project
        .board
        .setup
        .iter()
        .flat_map(|setup| setup.layers())
        .filter(|layer| layer.purpose == LayerPurpose::Copper)
        .map(|layer| layer.id)
        .collect();
    let mut ids = BTreeSet::new();
    for track in tracks.iter() {
        track.validate()?;
        if !ids.insert(track.id) {
            return Err("duplicate track identity".into());
        }
        if !copper.contains(&track.layer) {
            return Err("tracks must stay on a copper layer of the board setup".into());
        }
    }
    if !vias.is_empty() && copper.len() < 2 {
        return Err("vias need at least two copper layers in the board setup".into());
    }
    for via in vias.iter() {
        via.validate()?;
        if !ids.insert(via.id) {
            return Err("duplicate track or via identity".into());
        }
    }
    tracks.sort_by_key(|track| track.id);
    vias.sort_by_key(|via| via.id);
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
pub(super) struct Land {
    pub(super) key: (String, String),
    pub(super) corners: [Point; 4],
    pub(super) layers: Vec<Uuid>,
    pub(super) net: Option<String>,
    center: Point,
    shape: PadShape,
    corner_radius_nm: i64,
}

impl Land {
    /// Derive the copper contour from resolved corners and pad shape in board nanometers.
    pub(super) fn contour(&self) -> PadContour {
        if self.shape == PadShape::Rectangle {
            return PadContour { core: self.corners, radius: 0. };
        }
        let c = &self.corners;
        let (ux, uy) = ((c[1].x_nm - c[0].x_nm) as f64, (c[1].y_nm - c[0].y_nm) as f64);
        let (vx, vy) = ((c[3].x_nm - c[0].x_nm) as f64, (c[3].y_nm - c[0].y_nm) as f64);
        let (w, h) = (ux.hypot(uy), vx.hypot(vy));
        let radius = match self.shape {
            PadShape::Rectangle => 0.,
            PadShape::RoundedRectangle => (self.corner_radius_nm as f64).min(w.min(h) / 2.),
            PadShape::Circle | PadShape::Oval => w.min(h).round() / 2.,
        };
        let (x, y) = if self.shape == PadShape::RoundedRectangle {
            ((c[0].x_nm as f64 + c[2].x_nm as f64) / 2., (c[0].y_nm as f64 + c[2].y_nm as f64) / 2.)
        } else {
            (self.center.x_nm as f64, self.center.y_nm as f64)
        };
        let (iw, ih) = match self.shape {
            PadShape::Circle => (0., 0.),
            PadShape::Oval => ((w.round() / 2. - radius).max(0.), (h.round() / 2. - radius).max(0.)),
            _ => ((w / 2. - radius).max(0.), (h / 2. - radius).max(0.)),
        };
        let core = [(-iw, -ih), (iw, -ih), (iw, ih), (-iw, ih)].map(|(a, b)| Point {
            x_nm: (x + a * ux / w.max(1.) + b * vx / h.max(1.)).round() as i64,
            y_nm: (y + a * uy / w.max(1.) + b * vy / h.max(1.)).round() as i64,
        });
        PadContour { core, radius }
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
    /// no copper and stay isolated.
    pub fn copper(self) -> Copper {
        islands(&lands(self), self.tracks(), self.vias()).0
    }

    /// Where routed copper would go if footprint `id` took the given pose: the tracks whose ends
    /// ride on its pads, with those ends moved as `pcb.set_footprint_transform` would move them.
    /// For previews only; nothing is checked or edited, and an unknown ID carries nothing.
    pub fn carried_tracks(self, id: &str, position: Point, rotation: u16, bottom: bool) -> Vec<Track> {
        let Some(number) = crate::schematic::wiring::parse_id(id, "footprint-").filter(|_| !self.tracks().is_empty()) else {
            return Vec::new();
        };
        let mut next = self.project().clone();
        let board = std::sync::Arc::make_mut(&mut next.board);
        let Ok(index) = board.footprints.binary_search_by_key(&number, |f| f.id) else { return Vec::new() };
        let footprint = &mut board.footprints[index];
        (footprint.x_nm, footprint.y_nm, footprint.rotation, footprint.bottom) = (position.x_nm, position.y_nm, rotation, bottom);
        let Some(after) = next.board_view(self.target) else { return Vec::new() };
        carry(self, after, &BTreeSet::from([id.to_owned()]))
    }

    /// The net each track and via carries: the net of the first pad in its copper island. Copper
    /// that reaches no netted pad is absent. Nets are derived, never stored on the copper.
    pub fn copper_nets(self) -> BTreeMap<Uuid, String> {
        self.copper_with_nets().1
    }

    /// Resolve physical islands and the net of each netted track or via in one connectivity pass.
    pub fn copper_with_nets(self) -> (Copper, BTreeMap<Uuid, String>) {
        let lands = lands(self);
        let (tracks, vias) = (self.tracks(), self.vias());
        let (copper, members) = islands(&lands, tracks, vias);
        let island_net: Vec<Option<String>> =
            members.iter().map(|island| island.iter().find_map(|node| lands.get(*node).and_then(|land| land.net.clone()))).collect();
        let nets = copper.tracks.iter().chain(&copper.vias).filter_map(|(id, island)| island_net[*island].clone().map(|net| (*id, net))).collect();
        (copper, nets)
    }
}

/// Resolve each physical pad's land, layers and net once per snapshot. Markers without
/// physical copper and pads on an unconfigured board are omitted.
pub(super) fn lands(view: BoardView<'_>) -> Vec<Land> {
    view.footprints()
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
    let contours: Vec<_> = lands.iter().map(Land::contour).collect();
    let bounds: Vec<_> = contours.iter().map(PadContour::bounds).collect();
    let overlaps_pad = |i: usize, a: Point, b: Point, radius: f64| {
        let [left, top, right, bottom] = bounds[i];
        a.x_nm.min(b.x_nm) as f64 - radius <= right
            && a.x_nm.max(b.x_nm) as f64 + radius >= left
            && a.y_nm.min(b.y_nm) as f64 - radius <= bottom
            && a.y_nm.max(b.y_nm) as f64 + radius >= top
            && contours[i].overlaps_segment(a, b, radius)
    };
    let mut parent: Vec<usize> = (0..lands.len() + tracks.len() + vias.len()).collect();
    if join_pad_pairs {
        for_each_pad_contact(lands, &contours, &bounds, |i, j| join(&mut parent, i, j));
    }
    for (t, track) in tracks.iter().enumerate() {
        let node = lands.len() + t;
        for (l, land) in lands.iter().enumerate() {
            if land.layers.contains(&track.layer) && overlaps_pad(l, track.start, track.end, track.width_nm as f64 / 2.) {
                join(&mut parent, node, l);
            }
        }
        for (o, other) in tracks.iter().enumerate().take(t) {
            if other.layer == track.layer
                && PadContour::capsule(track.start, track.end, track.width_nm as f64 / 2.).touches(&PadContour::capsule(
                    other.start,
                    other.end,
                    other.width_nm as f64 / 2.,
                ))
            {
                join(&mut parent, node, lands.len() + o);
            }
        }
    }
    for (v, via) in vias.iter().enumerate() {
        let node = lands.len() + tracks.len() + v;
        let radius = via.diameter_nm as f64 / 2.;
        for (l, land) in lands.iter().enumerate() {
            if !land.layers.is_empty() && overlaps_pad(l, via.at, via.at, radius) {
                join(&mut parent, node, l);
            }
        }
        for (t, track) in tracks.iter().enumerate() {
            if PadContour::capsule(via.at, via.at, radius).touches(&PadContour::capsule(track.start, track.end, track.width_nm as f64 / 2.)) {
                join(&mut parent, node, lands.len() + t);
            }
        }
        for (o, other) in vias.iter().enumerate().take(v) {
            if PadContour::capsule(via.at, via.at, radius).touches(&PadContour::capsule(other.at, other.at, other.diameter_nm as f64 / 2.)) {
                join(&mut parent, node, lands.len() + tracks.len() + o);
            }
        }
    }
    let mut roots = BTreeMap::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    let mut copper = Copper::default();
    for node in 0..parent.len() {
        let root = find(&mut parent, node);
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
    let (_, members) = build_islands(&lands, tracks, vias, !footprints.is_empty());
    let is_added = |node: usize| {
        if let Some(land) = lands.get(node) {
            return footprints.contains(&land.key.0);
        }
        let index = node.checked_sub(lands.len());
        let track = index.and_then(|index| tracks.get(index)).map(|track| track.id);
        let via = index.and_then(|index| index.checked_sub(tracks.len())).and_then(|index| vias.get(index)).map(|via| via.id);
        track.or(via).is_some_and(|id| added.contains(&id))
    };
    members.iter().filter(|island| island.iter().any(|node| is_added(*node))).find_map(|island| {
        let mut nets = island.iter().filter_map(|node| lands.get(*node)?.net.clone());
        let first = nets.next()?;
        nets.find(|net| *net != first).map(|second| (first, second))
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
    let old: Vec<_> = lands(before)
        .into_iter()
        .filter(|land| moved.contains(&land.key.0))
        .map(|land| {
            let contour = land.contour();
            (land, contour)
        })
        .collect();
    if old.is_empty() {
        return Vec::new();
    }
    let new: BTreeMap<(String, String), Land> =
        lands(after).into_iter().filter(|land| moved.contains(&land.key.0)).map(|land| (land.key.clone(), land)).collect();
    let follow = |end: Point, track: &Track| {
        let half = track.width_nm as f64 / 2.;
        let Some((land, _)) = old.iter().find(|(land, contour)| land.layers.contains(&track.layer) && contour.overlaps_segment(end, end, half))
        else {
            return end;
        };
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
    /// board, revision and undo history unchanged.
    pub(crate) fn edit_tracks(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let mut tracks = project.board.tracks.clone();
        let mut vias = project.board.vias.clone();
        let mut added = BTreeSet::new();
        if command == ADD_TRACKS {
            let mut layer: Uuid = param(params, "layer")?;
            let width_nm: i64 = param(params, "width_nm")?;
            let points: Vec<Point> = param(params, "points")?;
            if !(2..=MAX_PATH_POINTS).contains(&points.len()) || points.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(CommandError::InvalidParams(format!("a route needs 2 to {MAX_PATH_POINTS} points with distinct neighbors")));
            }
            let changes: Vec<LayerChange> = if params.get("layer_changes").is_some() { param(params, "layer_changes")? } else { Vec::new() };
            let copper: BTreeSet<_> = project
                .board
                .setup
                .iter()
                .flat_map(|setup| setup.layers())
                .filter(|record| record.purpose == LayerPurpose::Copper)
                .map(|record| record.id)
                .collect();
            let mut previous = 0;
            for change in &changes {
                if change.at <= previous || change.at >= points.len() {
                    return Err(CommandError::InvalidParams("layer changes need strictly increasing vertex indices after the first".into()));
                }
                if !copper.contains(&change.layer) {
                    return Err(CommandError::InvalidParams("a layer change must move to a copper layer of the board setup".into()));
                }
                previous = change.at;
            }
            let size =
                |name| -> Result<i64, CommandError> { if changes.is_empty() && params.get(name).is_none() { Ok(0) } else { param(params, name) } };
            let (diameter_nm, drill_nm) = (size("via_diameter_nm")?, size("via_drill_nm")?);
            let mut changes = changes.iter().peekable();
            for (index, pair) in points.windows(2).enumerate() {
                if let Some(change) = changes.next_if(|change| change.at == index) {
                    if change.layer == layer {
                        return Err(CommandError::InvalidParams("a layer change must move to a different copper layer".into()));
                    }
                    layer = change.layer;
                    let id = crate::fresh_id()?;
                    added.insert(id);
                    vias.push(Via { id, locked: false, at: pair[0], diameter_nm, drill_nm });
                }
                let id = crate::fresh_id()?;
                added.insert(id);
                tracks.push(Track { id, locked: false, layer, width_nm, start: pair[0], end: pair[1] });
            }
            // A change at the final vertex ends the route on a via, ready to continue on that layer.
            if let Some(change) = changes.next() {
                if change.layer == layer {
                    return Err(CommandError::InvalidParams("a layer change must move to a different copper layer".into()));
                }
                let id = crate::fresh_id()?;
                added.insert(id);
                vias.push(Via { id, locked: false, at: points[change.at], diameter_nm, drill_nm });
            }
        } else if command == ADD_VIA {
            let at: Point = param(params, "at")?;
            if !crate::wire::valid_point(&at) {
                return Err(CommandError::InvalidParams("the via position is out of range".into()));
            }
            if vias.iter().any(|via| via.at == at) {
                return Err(CommandError::InvalidParams("a via already sits at this position".into()));
            }
            let id = crate::fresh_id()?;
            added.insert(id);
            vias.push(Via { id, locked: false, at, diameter_nm: param(params, "via_diameter_nm")?, drill_nm: param(params, "via_drill_nm")? });
        } else if command == MOVE_TRACKS {
            let moved_tracks = ids(params, "tracks", |id| tracks.iter().any(|track| track.id == *id))?;
            let moved_vias = ids(params, "vias", |id| vias.iter().any(|via| via.id == *id))?;
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
            for ends in &ends {
                let track =
                    tracks.iter_mut().find(|track| track.id == ends.track).ok_or_else(|| CommandError::InvalidParams("unknown track".into()))?;
                (track.start, track.end) = (ends.start, ends.end);
            }
            added.extend(unique);
        } else if command == SET_TRACK_WIDTH {
            let changed = ids(params, "tracks", |id| tracks.iter().any(|track| track.id == *id))?;
            if changed.is_empty() {
                return Err(CommandError::InvalidParams("name at least one track".into()));
            }
            let width_nm: i64 = param(params, "width_nm")?;
            for track in tracks.iter_mut().filter(|track| changed.contains(&track.id)) {
                track.width_nm = width_nm;
            }
            added.extend(changed);
        } else if command == SET_VIA_SIZE {
            let changed = ids(params, "vias", |id| vias.iter().any(|via| via.id == *id))?;
            if changed.is_empty() {
                return Err(CommandError::InvalidParams("name at least one via".into()));
            }
            let (diameter_nm, drill_nm): (i64, i64) = (param(params, "via_diameter_nm")?, param(params, "via_drill_nm")?);
            for via in vias.iter_mut().filter(|via| changed.contains(&via.id)) {
                (via.diameter_nm, via.drill_nm) = (diameter_nm, drill_nm);
            }
            added.extend(changed);
        } else {
            let gone_tracks = ids(params, "tracks", |id| tracks.iter().any(|track| track.id == *id))?;
            let gone_vias = ids(params, "vias", |id| vias.iter().any(|via| via.id == *id))?;
            if gone_tracks.is_empty() && gone_vias.is_empty() {
                return Err(CommandError::InvalidParams("name at least one track or via to delete".into()));
            }
            tracks.retain(|track| !gone_tracks.contains(&track.id));
            vias.retain(|via| !gone_vias.contains(&via.id));
        }
        validate(project, &mut tracks, &mut vias).map_err(CommandError::InvalidParams)?;
        if tracks == project.board.tracks && vias == project.board.vias {
            return Ok(());
        }
        let mut next = project.clone();
        let board = std::sync::Arc::make_mut(&mut next.board);
        board.tracks = tracks;
        board.vias = vias;
        if let Some((a, b)) = short(&next, &added) {
            return Err(CommandError::InvalidParams(format!("this track would connect nets {a} and {b}")));
        }
        self.change(next)?;
        Ok(())
    }
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
