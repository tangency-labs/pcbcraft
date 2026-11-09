//! Copper track segments on configured copper layers, and the copper connectivity they create.
//!
//! A track is one straight segment with an explicit width on one copper layer. Routing a path
//! adds several segments in one undo step. Connectivity is geometric: copper on the same layer
//! connects where it overlaps. This is a bounded connectivity aid for the ratsnest and for
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

/// Add a routed path of straight track segments on one copper layer, in one undo step.
pub const ADD_TRACKS: &str = "pcb.add_tracks";
/// Remove addressed track segments without changing footprints or other copper.
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
    fn validate(&self) -> Result<(), String> {
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

/// Validate every segment of a candidate board, then sort by ID for diff-stable saves.
/// Each layer must be a copper layer of the board's setup; an unconfigured board has none.
pub(crate) fn validate(project: &Project, tracks: &mut [Track]) -> Result<(), String> {
    if tracks.len() > MAX_TRACKS {
        return Err(format!("a board can hold at most {MAX_TRACKS} track segments"));
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
    tracks.sort_by_key(|track| track.id);
    Ok(())
}

/// Copper islands for one board snapshot: which pads and tracks are joined by same-layer copper.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Copper {
    /// Island index for each physical pad, keyed by footprint ID and pad number.
    pub pads: BTreeMap<(String, String), usize>,
    /// Island index for each track segment.
    pub tracks: BTreeMap<Uuid, usize>,
}

/// A pad reduced to what connectivity needs: its land corners, layers and net label.
struct Land {
    key: (String, String),
    corners: [Point; 4],
    layers: Vec<Uuid>,
    net: Option<String>,
}

impl<'a> BoardView<'a> {
    /// Authored track segments in stable ID order.
    pub fn tracks(self) -> &'a [Track] {
        &self.project().board.tracks
    }

    /// Group physical pads and tracks into islands joined by overlapping copper on a shared layer.
    /// Legacy center-only markers and unconfigured pads have no copper and stay isolated.
    pub fn copper(self) -> Copper {
        islands(&lands(self), self.tracks()).0
    }
}

/// Resolve each physical pad's land, layers and net once per snapshot. Markers without
/// physical copper and pads on an unconfigured board are omitted.
fn lands(view: BoardView<'_>) -> Vec<Land> {
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

/// Union-find over lands followed by tracks. Returns islands plus each island's member indices.
fn islands(lands: &[Land], tracks: &[Track]) -> (Copper, Vec<Vec<usize>>) {
    let mut parent: Vec<usize> = (0..lands.len() + tracks.len()).collect();
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
        } else {
            copper.tracks.insert(tracks[node - lands.len()].id, island);
        }
    }
    (copper, members)
}

/// Whether a track's copper reaches a convex pad land: an end inside it, or the land's center
/// within half the track width of the centerline. Corners arrive in boundary order.
fn touches_land(track: &Track, corners: &[Point; 4]) -> bool {
    let center = Point { x_nm: corners.iter().map(|p| p.x_nm).sum::<i64>() / 4, y_nm: corners.iter().map(|p| p.y_nm).sum::<i64>() / 4 };
    inside(track.start, corners)
        || inside(track.end, corners)
        || segment_distance(track.start, track.end, center, center) <= track.width_nm as f64 / 2.
}

/// Point-in-convex-polygon by consistent cross-product sign; boundary points count as inside.
fn inside(point: Point, corners: &[Point; 4]) -> bool {
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
fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let cross =
        |o: Point, p: Point, q: Point| (p.x_nm - o.x_nm) as i128 * (q.y_nm - o.y_nm) as i128 - (p.y_nm - o.y_nm) as i128 * (q.x_nm - o.x_nm) as i128;
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    if d1.signum() * d2.signum() < 0 && d3.signum() * d4.signum() < 0 {
        return 0.;
    }
    [point_distance(a, c, d), point_distance(b, c, d), point_distance(c, a, b), point_distance(d, a, b)].into_iter().fold(f64::INFINITY, f64::min)
}

/// Distance from a point to a closed segment; a degenerate segment is a point.
fn point_distance(p: Point, a: Point, b: Point) -> f64 {
    let (px, py, ax, ay, bx, by) = (p.x_nm as f64, p.y_nm as f64, a.x_nm as f64, a.y_nm as f64, b.x_nm as f64, b.y_nm as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx * dx + dy * dy;
    let t = if length > 0. { (((px - ax) * dx + (py - ay) * dy) / length).clamp(0., 1.) } else { 0. };
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

/// Name the first two distinct nets that a candidate's copper would join, if any.
fn short(project: &Project) -> Option<(String, String)> {
    let view = project.board_view(project.board_target()?)?;
    let lands = lands(view);
    let (_, members) = islands(&lands, view.tracks());
    members.iter().find_map(|island| {
        let mut nets = island.iter().filter_map(|node| lands.get(*node)?.net.clone());
        let first = nets.next()?;
        nets.find(|net| *net != first).map(|second| (first, second))
    })
}

/// Parse a JSON value into a typed parameter, reporting serde's message as invalid input.
fn param<T: serde::de::DeserializeOwned>(params: &Value, name: &str) -> Result<T, CommandError> {
    serde_json::from_value(params[name].clone()).map_err(|error| CommandError::InvalidParams(format!("{name}: {error}")))
}

impl Session {
    /// Add one routed path or delete addressed segments atomically. A path of N distinct
    /// consecutive points becomes N-1 segments on one copper layer with one width. The whole
    /// candidate is validated first: unknown layers or IDs, invalid geometry, and copper that
    /// would join two different nets leave the board, revision and undo history unchanged.
    pub(crate) fn edit_tracks(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let mut tracks = project.board.tracks.clone();
        if command == ADD_TRACKS {
            let layer: Uuid = param(params, "layer")?;
            let width_nm: i64 = param(params, "width_nm")?;
            let points: Vec<Point> = param(params, "points")?;
            if !(2..=MAX_PATH_POINTS).contains(&points.len()) || points.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(CommandError::InvalidParams(format!("a route needs 2 to {MAX_PATH_POINTS} points with distinct neighbors")));
            }
            for pair in points.windows(2) {
                tracks.push(Track { id: crate::fresh_id()?, layer, width_nm, start: pair[0], end: pair[1] });
            }
        } else {
            let ids: Vec<Uuid> = param(params, "tracks")?;
            let unique: BTreeSet<_> = ids.iter().collect();
            if ids.is_empty() || unique.len() != ids.len() || !ids.iter().all(|id| tracks.iter().any(|track| track.id == *id)) {
                return Err(CommandError::InvalidParams("tracks must be unique IDs of existing track segments".into()));
            }
            tracks.retain(|track| !unique.contains(&track.id));
        }
        validate(project, &mut tracks).map_err(CommandError::InvalidParams)?;
        let mut next = project.clone();
        std::sync::Arc::make_mut(&mut next.board).tracks = tracks;
        if command == ADD_TRACKS
            && let Some((a, b)) = short(&next)
        {
            return Err(CommandError::InvalidParams(format!("this track would connect nets {a} and {b}")));
        }
        self.change(next);
        Ok(())
    }
}

/// Describe a routed path's vertices for command clients.
pub(crate) fn path_schema() -> Value {
    let mut schema = super::drawing::geometry_schema();
    schema["maxItems"] = json!(MAX_PATH_POINTS);
    schema
}

#[cfg(test)]
mod tests;
