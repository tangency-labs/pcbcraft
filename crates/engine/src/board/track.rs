//! Copper track segments and through vias on configured copper layers, and the copper
//! connectivity they create.
//!
//! A track is one straight segment with an explicit width on one copper layer. A via is a
//! plated hole whose round pad spans every copper layer. Routing a path adds its segments and
//! vias in one undo step. Connectivity is geometric: copper on the same layer connects where it
//! overlaps, and a via joins whatever copper it overlaps on any layer. This is a bounded connectivity aid for the ratsnest and for
//! refusing shorts between nets. It is not a clearance check, DRC or manufacturing validation.
use super::{
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
    pub layer: Uuid,
    pub width_nm: i64,
    pub start: Point,
    pub end: Point,
}

impl Track {
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
    pub at: Point,
    pub diameter_nm: i64,
    pub drill_nm: i64,
}

impl Via {
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

/// A pad reduced to what connectivity needs: its land corners, layers and net label.
pub(super) struct Land {
    pub(super) key: (String, String),
    pub(super) corners: [Point; 4],
    pub(super) layers: Vec<Uuid>,
    pub(super) net: Option<String>,
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

    /// The net each track and via carries: the net of the first pad in its copper island. Copper
    /// that reaches no netted pad is absent. Nets are derived, never stored on the copper.
    pub fn copper_nets(self) -> BTreeMap<Uuid, String> {
        let lands = lands(self);
        let (tracks, vias) = (self.tracks(), self.vias());
        let (copper, members) = islands(&lands, tracks, vias);
        let island_net: Vec<Option<String>> =
            members.iter().map(|island| island.iter().find_map(|node| lands.get(*node).and_then(|land| land.net.clone()))).collect();
        copper.tracks.iter().chain(&copper.vias).filter_map(|(id, island)| island_net[*island].clone().map(|net| (*id, net))).collect()
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
                Some(Land { key: (id.clone(), pad.number), corners, layers, net: pad.net })
            })
        })
        .collect()
}

/// Union-find over lands, then tracks, then vias. Returns islands plus each island's member
/// node indices, numbered in that order.
pub(super) fn islands(lands: &[Land], tracks: &[Track], vias: &[Via]) -> (Copper, Vec<Vec<usize>>) {
    let mut parent: Vec<usize> = (0..lands.len() + tracks.len() + vias.len()).collect();
    fn find(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    let join = |parent: &mut Vec<usize>, a: usize, b: usize| {
        let (a, b) = (find(parent, a), find(parent, b));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    };
    for (t, track) in tracks.iter().enumerate() {
        let node = lands.len() + t;
        for (l, land) in lands.iter().enumerate() {
            if land.layers.contains(&track.layer) && touches_land(track, &land.corners) {
                join(&mut parent, node, l);
            }
        }
        for (o, other) in tracks.iter().enumerate().take(t) {
            if other.layer == track.layer
                && segment_distance(track.start, track.end, other.start, other.end) <= (track.width_nm + other.width_nm) as f64 / 2.
            {
                join(&mut parent, node, lands.len() + o);
            }
        }
    }
    for (v, via) in vias.iter().enumerate() {
        let node = lands.len() + tracks.len() + v;
        let radius = via.diameter_nm as f64 / 2.;
        for (l, land) in lands.iter().enumerate() {
            if !land.layers.is_empty() && via_touches_land(via.at, radius, &land.corners) {
                join(&mut parent, node, l);
            }
        }
        for (t, track) in tracks.iter().enumerate() {
            if point_distance(via.at, track.start, track.end) <= radius + track.width_nm as f64 / 2. {
                join(&mut parent, node, lands.len() + t);
            }
        }
        for (o, other) in vias.iter().enumerate().take(v) {
            if point_distance(via.at, other.at, other.at) <= radius + other.diameter_nm as f64 / 2. {
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

/// Whether a track's copper overlaps a convex pad land: an end inside it, or the centerline
/// within half the track width of any land edge. A centerline crossing an edge has distance
/// zero, so a track passing over the land off-center counts. Corners arrive in boundary order.
fn touches_land(track: &Track, corners: &[Point; 4]) -> bool {
    let half = track.width_nm as f64 / 2.;
    inside(track.start, corners)
        || inside(track.end, corners)
        || corners.iter().zip(corners.iter().cycle().skip(1)).any(|(a, b)| segment_distance(track.start, track.end, *a, *b) <= half)
}

/// Whether a via's round pad overlaps a convex land: its center inside the land, or within its
/// radius of any land edge. Corners arrive in boundary order.
fn via_touches_land(at: Point, radius: f64, corners: &[Point; 4]) -> bool {
    inside(at, corners) || corners.iter().zip(corners.iter().cycle().skip(1)).any(|(a, b)| point_distance(at, *a, *b) <= radius)
}

/// Point-in-convex-polygon by consistent cross-product sign; boundary points count as inside.
pub(super) fn inside(point: Point, corners: &[Point; 4]) -> bool {
    let mut sign = 0i8;
    for (a, b) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        let cross = (b.x_nm - a.x_nm) as i128 * (point.y_nm - a.y_nm) as i128 - (b.y_nm - a.y_nm) as i128 * (point.x_nm - a.x_nm) as i128;
        let side = cross.signum() as i8;
        if side != 0 {
            if sign != 0 && side != sign {
                return false;
            }
            sign = side;
        }
    }
    true
}

/// Shortest distance between two closed segments in nanometers; zero when they cross.
pub(super) fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let cross =
        |o: Point, p: Point, q: Point| (p.x_nm - o.x_nm) as i128 * (q.y_nm - o.y_nm) as i128 - (p.y_nm - o.y_nm) as i128 * (q.x_nm - o.x_nm) as i128;
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    if d1.signum() * d2.signum() < 0 && d3.signum() * d4.signum() < 0 {
        return 0.;
    }
    [point_distance(a, c, d), point_distance(b, c, d), point_distance(c, a, b), point_distance(d, a, b)].into_iter().fold(f64::INFINITY, f64::min)
}

/// Distance from a point to a closed segment; a degenerate segment is a point.
pub(super) fn point_distance(p: Point, a: Point, b: Point) -> f64 {
    let (px, py, ax, ay, bx, by) = (p.x_nm as f64, p.y_nm as f64, a.x_nm as f64, a.y_nm as f64, b.x_nm as f64, b.y_nm as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx * dx + dy * dy;
    let t = if length > 0. { (((px - ax) * dx + (py - ay) * dy) / length).clamp(0., 1.) } else { 0. };
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

/// Name the first two distinct nets joined by an island that contains one of the `added`
/// tracks or vias, if any. Islands without new copper are ignored, so a short made earlier by
/// another edit (such as moving a footprint onto a track) does not block unrelated routes.
fn short(project: &Project, added: &BTreeSet<Uuid>) -> Option<(String, String)> {
    short_for_paste(project, added, &BTreeSet::new())
}

/// Apply the existing copper-contact approximation to newly pasted tracks/vias and footprint
/// lands. Unrelated pre-existing islands remain outside this operation's short check.
pub(crate) fn short_for_paste(project: &Project, added: &BTreeSet<Uuid>, footprints: &BTreeSet<String>) -> Option<(String, String)> {
    let view = project.board_view(project.board_target()?)?;
    let lands = lands(view);
    let (tracks, vias) = (view.tracks(), view.vias());
    let (_, members) = islands(&lands, tracks, vias);
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
                    vias.push(Via { id, at: pair[0], diameter_nm, drill_nm });
                }
                let id = crate::fresh_id()?;
                added.insert(id);
                tracks.push(Track { id, layer, width_nm, start: pair[0], end: pair[1] });
            }
            // A change at the final vertex ends the route on a via, ready to continue on that layer.
            if let Some(change) = changes.next() {
                if change.layer == layer {
                    return Err(CommandError::InvalidParams("a layer change must move to a different copper layer".into()));
                }
                let id = crate::fresh_id()?;
                added.insert(id);
                vias.push(Via { id, at: points[change.at], diameter_nm, drill_nm });
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
            vias.push(Via { id, at, diameter_nm: param(params, "via_diameter_nm")?, drill_nm: param(params, "via_drill_nm")? });
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
        self.change(next);
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
