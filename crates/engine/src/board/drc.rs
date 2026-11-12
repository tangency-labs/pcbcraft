//! A bounded design rules check over one board snapshot.
//!
//! The check reads the board without editing it. It reports copper clearance between different
//! nets, minimum track width, via annular ring, unrouted connections, the board outline (missing,
//! copper outside it, copper too close to its edge) and overlapping footprint courtyards. Rule
//! values are explicit parameters with documented defaults until net classes and board rules
//! exist (#77). Results are deterministic, so the same board and rules give the same report.
//! This is not a manufacturing sign-off: arcs, zones, hole-to-hole spacing, silkscreen and
//! solder mask checks are not implemented.
use super::{
    definition::GraphicPurpose,
    setup::LayerPurpose,
    track::{Land, PadContour, islands, lands, segment_distance},
    view::BoardView,
};
use crate::{CommandError, Session, schematic::Point};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Check the addressed board against explicit rules; returns a report and edits nothing.
pub const CHECK_DESIGN: &str = "pcb.check_design";
/// Copper-to-copper clearance between different nets used when none is given, 0.2 mm.
pub const DEFAULT_CLEARANCE_NM: i64 = 200_000;
/// Narrowest track allowed when no minimum is given, 0.15 mm.
pub const DEFAULT_MIN_TRACK_WIDTH_NM: i64 = 150_000;
/// Copper-to-board-edge clearance used when none is given, 0.3 mm.
pub const DEFAULT_EDGE_CLEARANCE_NM: i64 = 300_000;
/// Smallest via annular ring allowed when none is given, 0.1 mm.
pub const DEFAULT_MIN_ANNULAR_RING_NM: i64 = 100_000;
/// Violations listed per report; the counts still cover every finding.
const MAX_LISTED: usize = 1000;

/// The rule values one check used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rules {
    pub clearance_nm: i64,
    pub min_track_width_nm: i64,
    pub edge_clearance_nm: i64,
    pub min_annular_ring_nm: i64,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            clearance_nm: DEFAULT_CLEARANCE_NM,
            min_track_width_nm: DEFAULT_MIN_TRACK_WIDTH_NM,
            edge_clearance_nm: DEFAULT_EDGE_CLEARANCE_NM,
            min_annular_ring_nm: DEFAULT_MIN_ANNULAR_RING_NM,
        }
    }
}

/// What a violation is about. The serialized names are stable for MCP clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Clearance,
    TrackWidth,
    AnnularRing,
    Unconnected,
    NoOutline,
    OutsideBoard,
    EdgeClearance,
    CourtyardOverlap,
}

/// One finding: what, where (a representative point in board nanometers) and which items.
/// Items are footprint IDs (`footprint-N`), pads (`footprint-N:pad`) or track/via UUIDs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub kind: Kind,
    pub message: String,
    pub at: Point,
    pub items: Vec<String>,
}

/// The result of one check. `violations` is capped; `counts` covers every finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    pub rules: Rules,
    pub violations: Vec<Violation>,
    pub counts: BTreeMap<Kind, usize>,
    pub total: usize,
}

/// Copper reduced to a shape on some layers. `layers` empty means every copper layer (a via).
struct Item {
    name: String,
    shape: Shape,
    layers: Vec<Uuid>,
    island: usize,
    net: Option<String>,
    at: Point,
}

enum Shape {
    Segment { a: Point, b: Point, half: f64 },
    Circle { center: Point, radius: f64 },
    Pad(PadContour),
}

impl Shape {
    /// Axis-aligned bounds in nanometers.
    fn bounds(&self) -> [f64; 4] {
        match self {
            Shape::Segment { a, b, half } => [
                a.x_nm.min(b.x_nm) as f64 - half,
                a.y_nm.min(b.y_nm) as f64 - half,
                a.x_nm.max(b.x_nm) as f64 + half,
                a.y_nm.max(b.y_nm) as f64 + half,
            ],
            Shape::Circle { center, radius } => {
                [center.x_nm as f64 - radius, center.y_nm as f64 - radius, center.x_nm as f64 + radius, center.y_nm as f64 + radius]
            }
            Shape::Pad(contour) => contour.bounds(),
        }
    }

    /// The shape's outline as segments with the copper extending `half` either side; a circle
    /// is a zero-length segment with its radius. Pad core edges carry the existing fractional
    /// rounding radius; containment in a core with area is handled separately.
    fn segments(&self) -> Vec<(Point, Point, f64)> {
        match self {
            Shape::Segment { a, b, half } => vec![(*a, *b, *half)],
            Shape::Circle { center, radius } => vec![(*center, *center, *radius)],
            Shape::Pad(contour) => contour.edges().map(|(a, b)| (a, b, contour.radius)).collect(),
        }
    }

    /// Whether a skeleton point is contained in a pad's core with area. Degenerate point/line
    /// cores have no interior; their contact and clearance come from edge distances and radii.
    fn inside_pad(&self, pad: &PadContour) -> bool {
        match self {
            Shape::Segment { a, b, .. } => pad.contains_core(*a) || pad.contains_core(*b),
            Shape::Circle { center, .. } => pad.contains_core(*center),
            Shape::Pad(contour) => contour.core.iter().any(|point| pad.contains_core(*point)),
        }
    }
}

/// Copper-to-copper gap in nanometers; zero or negative means the shapes touch or overlap.
fn gap(a: &Shape, b: &Shape) -> f64 {
    if let Shape::Pad(pad) = b
        && a.inside_pad(pad)
    {
        return 0.;
    }
    if let Shape::Pad(pad) = a
        && b.inside_pad(pad)
    {
        return 0.;
    }
    let mut best = f64::INFINITY;
    for (a1, a2, ha) in a.segments() {
        for (b1, b2, hb) in b.segments() {
            best = best.min(segment_distance(a1, a2, b1, b2) - ha - hb);
        }
    }
    best
}

/// Whether two items share a copper layer; a via shares every layer.
fn share_layer(a: &Item, b: &Item) -> bool {
    a.layers.is_empty() || b.layers.is_empty() || a.layers.iter().any(|layer| b.layers.contains(layer))
}

/// Ray-casting point-in-polygon for a closed outline; boundary points count as outside.
fn in_polygon(point: Point, polygon: &[Point]) -> bool {
    let (x, y) = (point.x_nm as f64, point.y_nm as f64);
    let mut inside = false;
    for (i, a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        let (ax, ay, bx, by) = (a.x_nm as f64, a.y_nm as f64, b.x_nm as f64, b.y_nm as f64);
        if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
            inside = !inside;
        }
    }
    inside
}

/// The midpoint of two points, as a violation marker.
fn midpoint(a: Point, b: Point) -> Point {
    Point { x_nm: a.x_nm / 2 + b.x_nm / 2, y_nm: a.y_nm / 2 + b.y_nm / 2 }
}

/// Format a length for messages in millimeters, independent of the viewer's display units.
fn mm(nm: f64) -> String {
    format!("{:.3} mm", nm / 1_000_000.)
}

impl BoardView<'_> {
    /// Check this board against `rules` and return every finding, sorted by kind and position.
    pub fn check_design(self, rules: Rules) -> Report {
        let mut found = Vec::new();
        let lands = lands(self);
        let (tracks, vias) = (self.tracks(), self.vias());
        let (copper, members) = islands(&lands, tracks, vias);
        let island_net: Vec<Option<String>> =
            members.iter().map(|island| island.iter().find_map(|node| lands.get(*node).and_then(|land| land.net.clone()))).collect();
        let mut items = Vec::new();
        for land in &lands {
            let island = copper.pads[&land.key];
            items.push(Item {
                name: format!("{}:{}", land.key.0, land.key.1),
                shape: Shape::Pad(land.contour()),
                layers: land.layers.clone(),
                island,
                net: land.net.clone().or_else(|| island_net[island].clone()),
                at: center(land),
            });
        }
        for track in tracks {
            let island = copper.tracks[&track.id];
            items.push(Item {
                name: track.id.to_string(),
                shape: Shape::Segment { a: track.start, b: track.end, half: track.width_nm as f64 / 2. },
                layers: vec![track.layer],
                island,
                net: island_net[island].clone(),
                at: midpoint(track.start, track.end),
            });
            if track.width_nm < rules.min_track_width_nm {
                found.push(Violation {
                    kind: Kind::TrackWidth,
                    message: format!("Track is {} wide, below the {} minimum", mm(track.width_nm as f64), mm(rules.min_track_width_nm as f64)),
                    at: midpoint(track.start, track.end),
                    items: vec![track.id.to_string()],
                });
            }
        }
        for via in vias {
            let island = copper.vias[&via.id];
            items.push(Item {
                name: via.id.to_string(),
                shape: Shape::Circle { center: via.at, radius: via.diameter_nm as f64 / 2. },
                layers: Vec::new(),
                island,
                net: island_net[island].clone(),
                at: via.at,
            });
            let ring = (via.diameter_nm - via.drill_nm) / 2;
            if ring < rules.min_annular_ring_nm {
                found.push(Violation {
                    kind: Kind::AnnularRing,
                    message: format!("Via annular ring is {}, below the {} minimum", mm(ring as f64), mm(rules.min_annular_ring_nm as f64)),
                    at: via.at,
                    items: vec![via.id.to_string()],
                });
            }
        }

        // Clearance between copper on different nets that shares a layer. Items in one copper
        // island are connected, and items on the same named net need no clearance.
        let clearance = rules.clearance_nm as f64;
        let bounds: Vec<_> = items.iter().map(|item| item.shape.bounds()).collect();
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by(|a, b| bounds[*a][0].total_cmp(&bounds[*b][0]));
        for (position, &i) in order.iter().enumerate() {
            for &j in &order[position + 1..] {
                if bounds[j][0] > bounds[i][2] + clearance {
                    break;
                }
                let (a, b) = (&items[i], &items[j]);
                if a.island == b.island
                    || (a.net.is_some() && a.net == b.net)
                    || bounds[j][1] > bounds[i][3] + clearance
                    || bounds[i][1] > bounds[j][3] + clearance
                    || !share_layer(a, b)
                {
                    continue;
                }
                let distance = gap(&a.shape, &b.shape);
                if distance < clearance {
                    let nets = match (&a.net, &b.net) {
                        (Some(x), Some(y)) => format!(" between nets {x} and {y}"),
                        _ => String::new(),
                    };
                    let message = if distance <= 0. {
                        format!("Copper touches{nets}")
                    } else {
                        format!("Clearance {} is below the {} rule{nets}", mm(distance), mm(clearance))
                    };
                    let mut names = vec![a.name.clone(), b.name.clone()];
                    names.sort();
                    found.push(Violation { kind: Kind::Clearance, message, at: midpoint(a.at, b.at), items: names });
                }
            }
        }

        if let Some(ratsnest) = self.ratsnest() {
            for line in &ratsnest.lines {
                found.push(Violation {
                    kind: Kind::Unconnected,
                    message: "Unrouted connection".into(),
                    at: midpoint(line.from.position, line.to.position),
                    items: vec![
                        format!("{}:{}", line.from.footprint, line.from.pad.number.as_str()),
                        format!("{}:{}", line.to.footprint, line.to.pad.number.as_str()),
                    ],
                });
            }
        }

        self.check_outline(&items, rules, &mut found);
        self.check_courtyards(&mut found);

        found.sort_by(|a, b| (a.kind, a.at.x_nm, a.at.y_nm, &a.items).cmp(&(b.kind, b.at.x_nm, b.at.y_nm, &b.items)));
        let mut counts = BTreeMap::new();
        for violation in &found {
            *counts.entry(violation.kind).or_insert(0) += 1;
        }
        let total = found.len();
        found.truncate(MAX_LISTED);
        Report { rules, violations: found, counts, total }
    }

    /// Require a closed Board Outline, every copper item inside it, and copper at least the edge
    /// clearance from every outline segment. Open outline paths still count as edges.
    fn check_outline(self, items: &[Item], rules: Rules, found: &mut Vec<Violation>) {
        let layers = self.layers();
        let outline: Vec<_> = self
            .drawings()
            .into_iter()
            .filter(|drawing| layers.iter().any(|layer| layer.id == drawing.layer && layer.purpose == LayerPurpose::Outline))
            .collect();
        let polygons: Vec<_> = outline.iter().filter(|drawing| drawing.closed).map(|drawing| drawing.points.clone()).collect();
        if polygons.is_empty() {
            if !items.is_empty() || !outline.is_empty() {
                found.push(Violation {
                    kind: Kind::NoOutline,
                    message: "The board has no closed Board Outline".into(),
                    at: Point { x_nm: 0, y_nm: 0 },
                    items: Vec::new(),
                });
            }
            return;
        }
        let edges: Vec<(Point, Point)> = outline
            .iter()
            .flat_map(|drawing| {
                let points = &drawing.points;
                let count = if drawing.closed { points.len() } else { points.len().saturating_sub(1) };
                (0..count).map(move |i| (points[i], points[(i + 1) % points.len()]))
            })
            .collect();
        for item in items {
            let anchors: Vec<Point> = match &item.shape {
                Shape::Segment { a, b, .. } => vec![*a, *b],
                Shape::Circle { center, .. } => vec![*center],
                Shape::Pad(contour) => contour.core.to_vec(),
            };
            if !anchors.iter().all(|point| polygons.iter().any(|polygon| in_polygon(*point, polygon))) {
                found.push(Violation {
                    kind: Kind::OutsideBoard,
                    message: "Copper lies outside the Board Outline".into(),
                    at: item.at,
                    items: vec![item.name.clone()],
                });
                continue;
            }
            let distance = edges.iter().map(|(a, b)| gap(&item.shape, &Shape::Segment { a: *a, b: *b, half: 0. })).fold(f64::INFINITY, f64::min);
            if distance < rules.edge_clearance_nm as f64 {
                found.push(Violation {
                    kind: Kind::EdgeClearance,
                    message: format!("Copper is {} from the board edge, below the {} rule", mm(distance.max(0.)), mm(rules.edge_clearance_nm as f64)),
                    at: item.at,
                    items: vec![item.name.clone()],
                });
            }
        }
    }

    /// Report footprints on the same side whose courtyard bounds overlap. Courtyards are compared
    /// by their axis-aligned bounds, so rotated or concave courtyards can be reported conservatively.
    fn check_courtyards(self, found: &mut Vec<Violation>) {
        let courtyards: Vec<(String, bool, [i64; 4])> = self
            .footprints()
            .filter_map(|footprint| {
                let points: Vec<_> = footprint
                    .graphics
                    .iter()
                    .filter(|graphic| graphic.purpose == GraphicPurpose::Courtyard)
                    .flat_map(|graphic| graphic.points.iter().copied())
                    .collect();
                let first = points.first()?;
                let bounds = points.iter().fold([first.x_nm, first.y_nm, first.x_nm, first.y_nm], |b, p| {
                    [b[0].min(p.x_nm), b[1].min(p.y_nm), b[2].max(p.x_nm), b[3].max(p.y_nm)]
                });
                Some((footprint.id, footprint.bottom, bounds))
            })
            .collect();
        for (i, (a, side_a, ba)) in courtyards.iter().enumerate() {
            for (b, _, bb) in courtyards[i + 1..].iter().filter(|(_, side_b, _)| side_b == side_a) {
                if ba[0] < bb[2] && bb[0] < ba[2] && ba[1] < bb[3] && bb[1] < ba[3] {
                    let at = Point { x_nm: (ba[0].max(bb[0]) + ba[2].min(bb[2])) / 2, y_nm: (ba[1].max(bb[1]) + ba[3].min(bb[3])) / 2 };
                    found.push(Violation {
                        kind: Kind::CourtyardOverlap,
                        message: "Footprint courtyards overlap".into(),
                        at,
                        items: vec![a.clone(), b.clone()],
                    });
                }
            }
        }
    }
}

/// The center of a land, as its marker position.
fn center(land: &Land) -> Point {
    let c = &land.corners;
    Point { x_nm: c.iter().map(|p| p.x_nm).sum::<i64>() / 4, y_nm: c.iter().map(|p| p.y_nm).sum::<i64>() / 4 }
}

/// Read optional rule overrides; each must be a positive length within the coordinate range.
pub(crate) fn rules(params: &Value) -> Result<Rules, CommandError> {
    let mut rules = Rules::default();
    for (name, value) in [
        ("clearance_nm", &mut rules.clearance_nm),
        ("min_track_width_nm", &mut rules.min_track_width_nm),
        ("edge_clearance_nm", &mut rules.edge_clearance_nm),
        ("min_annular_ring_nm", &mut rules.min_annular_ring_nm),
    ] {
        if let Some(given) = params.get(name) {
            *value = given
                .as_i64()
                .filter(|n| (0..=crate::schematic::MAX_CENTER_NM).contains(n))
                .ok_or_else(|| CommandError::InvalidParams(format!("{name} must be a length from 0 to the board coordinate range")))?;
        }
    }
    Ok(rules)
}

impl Session {
    /// Validate the board address and rules, then report findings without editing anything.
    pub fn check_design(&self, params: &Value) -> Result<Value, CommandError> {
        let (view, rules) = self.design_check_inputs(params)?;
        Ok(json!(view.check_design(rules)))
    }

    /// Refuse a design check's bad board address or rules without running the check, so the
    /// shared command path validates once and only the response builds the report.
    pub(crate) fn validate_design_check(&self, params: &Value) -> Result<(), CommandError> {
        self.design_check_inputs(params).map(drop)
    }

    /// The board view and rule overrides a design check reads, with the check's errors.
    fn design_check_inputs(&self, params: &Value) -> Result<(BoardView<'_>, Rules), CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
        Ok((view, rules(params)?))
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod contour_tests;
