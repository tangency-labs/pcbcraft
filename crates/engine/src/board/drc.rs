//! A bounded design rules check over one board snapshot.
//!
//! The check reads the board without editing it. It reports copper clearance between different
//! nets, conflicting intended nets within a copper island, minimum track width, via annular ring,
//! unrouted connections, the board outline (missing, copper outside it, copper too close to its
//! edge) and overlapping footprint courtyards. Rule
//! values resolve saved board defaults and bounded request-local overrides through the shared resolver. Results are deterministic, so the same board and rules give the same report.
//! This is not a manufacturing sign-off: arcs, zones, hole-to-hole spacing, silkscreen and
//! solder mask checks are not implemented.
use super::{
    clearance::{BoardGeometry, Object as Item},
    definition::GraphicPurpose,
    geometry::Contour,
    setup::LayerPurpose,
    track::Land,
    update::TerminalKey,
    view::BoardView,
};
use crate::{CommandError, Session, schematic::Point};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[cfg(test)]
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

/// The shared effective board defaults; hard limits and preferences have distinct fields.
pub use super::rules::Values as Rules;

/// What a violation is about. The serialized names are stable for MCP clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// One physical copper island joins pads from distinct intended nets, independently of
    /// the clearance rule, including when the required clearance is zero.
    Short,
    Clearance,
    TrackWidth,
    ViaDiameter,
    ViaDrill,
    AnnularRing,
    Unconnected,
    NoOutline,
    OutsideBoard,
    EdgeClearance,
    CourtyardOverlap,
}

/// Intended membership participating in a short. `net` is an index in this snapshot's accepted
/// baseline, or its project-wide logical graph when no baseline exists, never a persisted ID.
/// Names are display text only; component-terminal keys identify the involved logical objects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShortNet {
    /// Index in the checked snapshot's accepted baseline or logical graph, never a saved net ID.
    pub net: usize,
    /// Display label for this membership; equal labels do not imply equal nets.
    pub name: String,
    /// Involved logical terminals, sorted and deduplicated by component identity and pin text.
    pub terminals: Vec<TerminalKey>,
}

/// One finding: what, where (a representative point in board nanometers) and which items.
/// Items are footprint IDs (`footprint-N`), pads (`footprint-N:pad`) or track/via UUIDs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub kind: Kind,
    pub message: String,
    pub at: Point,
    pub items: Vec<String>,
    /// Conflicting memberships for short findings, absent from other kinds. Copper item
    /// references above and logical terminals here belong to the checked board snapshot.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<ShortNet>,
}

/// The result of one check. `violations` is capped; `counts` covers every finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    pub rules: Rules,
    /// Per-property built-in, saved board-rule or request-local provenance.
    pub sources: BTreeMap<String, super::rules::Source>,
    pub violations: Vec<Violation>,
    pub counts: BTreeMap<Kind, usize>,
    pub total: usize,
}

/// Count every finding while retaining only the first canonical report page. The sequence
/// breaks equal-key ties in discovery order, matching the synchronous stable sort.
#[derive(Default)]
struct Findings {
    listed: BTreeMap<(Kind, i64, i64, Vec<String>, usize), Violation>,
    counts: BTreeMap<Kind, usize>,
    total: usize,
}

impl Findings {
    /// Retain bounded detail without losing totals; a missing outline is one board finding.
    fn push(&mut self, violation: Violation) {
        if violation.kind == Kind::NoOutline && self.counts.contains_key(&Kind::NoOutline) {
            return;
        }
        let key = (violation.kind, violation.at.x_nm, violation.at.y_nm, violation.items.clone(), self.total);
        *self.counts.entry(violation.kind).or_default() += 1;
        self.total += 1;
        if self.listed.len() < MAX_LISTED || self.listed.last_key_value().is_some_and(|(last, _)| key < *last) {
            self.listed.insert(key, violation);
            if self.listed.len() > MAX_LISTED {
                self.listed.pop_last();
            }
        }
    }
    /// Produce the deterministic capped report after every phase has completed.
    fn finish(self, rules: Rules, sources: BTreeMap<String, super::rules::Source>) -> Report {
        Report { rules, sources, violations: self.listed.into_values().collect(), counts: self.counts, total: self.total }
    }
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
        let mut found = Findings::default();
        let geometry = BoardGeometry::new(self);
        let items = &geometry.items;
        self.check_dimensions(rules, &mut found);

        self.check_shorts(&geometry.lands, &geometry.members, items, &mut found);

        // Physical island short diagnostics remain independent of the strict clearance rule.
        let clearance = rules.clearance_nm as f64;
        for decision in geometry.clearance_pairs(rules.clearance_nm) {
            if !decision.clearance_violation {
                continue;
            }
            if let Some(i) = decision.first {
                let (a, b) = (&items[i], &items[decision.second]);
                let distance = decision.gap_nm;
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
                found.push(Violation { kind: Kind::Clearance, message, at: midpoint(a.at, b.at), items: names, nets: Vec::new() });
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
                    nets: Vec::new(),
                });
            }
        }

        self.check_outline(items, rules, &mut found);
        self.check_courtyards(&mut found);

        let mut effective = self.effective_rules();
        let supplied = json!(rules);
        let base = json!(effective.values);
        for name in super::rules::LIMIT_FIELDS.iter().chain(super::rules::PREFERENCE_FIELDS) {
            if supplied[*name] != base[*name] {
                effective.sources.insert((*name).into(), super::rules::Source::Request);
            }
        }
        found.finish(rules, effective.sources)
    }

    /// Check authored track/via dimensions without preparing connectivity or changing state.
    fn check_dimensions(self, rules: Rules, found: &mut Findings) {
        for track in self.tracks() {
            if track.width_nm < rules.min_track_width_nm {
                found.push(Violation {
                    kind: Kind::TrackWidth,
                    message: format!("Track is {} wide, below the {} minimum", mm(track.width_nm as f64), mm(rules.min_track_width_nm as f64)),
                    at: midpoint(track.start, track.end),
                    items: vec![track.id.to_string()],
                    nets: Vec::new(),
                });
            }
        }
        for via in self.vias() {
            for (kind, value, min, max, name) in [
                (Kind::ViaDiameter, via.diameter_nm, rules.min_via_diameter_nm, rules.max_via_diameter_nm, "Via diameter"),
                (Kind::ViaDrill, via.drill_nm, rules.min_via_drill_nm, rules.max_via_drill_nm, "Via drill"),
            ] {
                if !(min..=max).contains(&value) {
                    found.push(Violation {
                        kind,
                        message: format!("{name} is {}, outside {} to {}", mm(value as f64), mm(min as f64), mm(max as f64)),
                        at: via.at,
                        items: vec![via.id.to_string()],
                        nets: Vec::new(),
                    });
                }
            }
            let ring = (via.diameter_nm - via.drill_nm) / 2;
            if ring < rules.min_annular_ring_nm {
                found.push(Violation {
                    kind: Kind::AnnularRing,
                    message: format!("Via annular ring is {}, below the {} minimum", mm(ring as f64), mm(rules.min_annular_ring_nm as f64)),
                    at: via.at,
                    items: vec![via.id.to_string()],
                    nets: Vec::new(),
                });
            }
        }
    }

    /// Report one short per physical island containing pads from distinct intended nets.
    /// Resolve terminal-to-pad mappings against accepted history when present, including retained
    /// removals; otherwise use the project-wide graph. All indices and lookups stay snapshot-local.
    /// Every involved copper item and mapped terminal is retained, in canonical identity order.
    fn check_shorts(self, lands: &[Land], members: &[Vec<usize>], items: &[Item], found: &mut Findings) {
        let project = self.project();
        let baseline = project.board.baseline.as_ref();
        let accepted = baseline.map(|baseline| baseline.terminal_net_index());
        let graph = baseline.is_none().then(|| project.electrical_connectivity());
        let memberships: Vec<_> = lands
            .iter()
            .map(|land| {
                let pad = super::Pad::parse(&land.key.0, &land.key.1)?;
                let (component, terminal) = project.board.component_pin_for(pad)?;
                let net = if let Some(accepted) = &accepted {
                    *accepted.get(&(component, terminal))?
                } else {
                    graph.as_ref()?.net_index_for(project.component_terminal(component, terminal)?)?
                };
                Some((net, TerminalKey { component, terminal: terminal.to_owned() }))
            })
            .collect();
        for island in members {
            let mut nets = BTreeMap::<usize, Vec<TerminalKey>>::new();
            for &node in island {
                if let Some(Some((net, terminal))) = memberships.get(node) {
                    nets.entry(*net).or_default().push(terminal.clone());
                }
            }
            if nets.len() < 2 {
                continue;
            }
            let nets: Vec<_> = nets
                .into_iter()
                .map(|(net, mut terminals)| {
                    terminals.sort();
                    terminals.dedup();
                    let name = baseline.and_then(|baseline| baseline.net_name(net)).or_else(|| graph.as_ref().map(|graph| graph.net_name(net)));
                    ShortNet { net, name: name.unwrap_or_default(), terminals }
                })
                .collect();
            let names: Vec<_> = nets.iter().map(|net| format!("{} (net {})", net.name, net.net + 1)).collect();
            let at = island.iter().find_map(|&node| memberships.get(node)?.as_ref().map(|_| items[node].at));
            let mut names_of_items: Vec<_> = island.iter().map(|&node| items[node].name.clone()).collect();
            names_of_items.sort();
            if let Some(at) = at {
                found.push(Violation {
                    kind: Kind::Short,
                    message: format!("Copper island connects intended nets {}", names.join(", ")),
                    at,
                    items: names_of_items,
                    nets,
                });
            }
        }
    }

    /// Require a closed Board Outline, every copper item inside it, and copper at least the edge
    /// clearance from every outline segment. Penetration remains a finding at zero required
    /// clearance. Open outline paths still count as edges.
    fn check_outline(self, items: &[Item], rules: Rules, found: &mut Findings) {
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
                    nets: Vec::new(),
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
            let anchors = &item.contour.core;
            if !anchors.iter().all(|point| polygons.iter().any(|polygon| in_polygon(*point, polygon))) {
                found.push(Violation {
                    kind: Kind::OutsideBoard,
                    message: "Copper lies outside the Board Outline".into(),
                    at: item.at,
                    items: vec![item.name.clone()],
                    nets: Vec::new(),
                });
                continue;
            }
            let (distance, crosses) = edges.iter().fold((f64::INFINITY, false), |(best, crosses), (a, b)| {
                let distance = item.contour.gap(&Contour::capsule(*a, *b, 0.));
                // A contained edge can have gap zero even though it enters a pad's copper.
                let penetrates = distance < 0. || (distance == 0. && item.contour.penetrates_segment(*a, *b));
                (best.min(distance), crosses || penetrates)
            });
            if crosses || distance < rules.edge_clearance_nm as f64 {
                found.push(Violation {
                    kind: Kind::EdgeClearance,
                    message: if crosses {
                        "Copper crosses the Board Outline".into()
                    } else {
                        format!("Copper is {} from the board edge, below the {} rule", mm(distance.max(0.)), mm(rules.edge_clearance_nm as f64))
                    },
                    at: item.at,
                    items: vec![item.name.clone()],
                    nets: Vec::new(),
                });
            }
        }
    }

    /// Report footprints on the same side whose courtyard bounds overlap. Courtyards are compared
    /// by their axis-aligned bounds, so rotated or concave courtyards can be reported conservatively.
    fn check_courtyards(self, found: &mut Findings) {
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
                        nets: Vec::new(),
                    });
                }
            }
        }
    }
}

impl Session {
    /// Validate the board address and rules, then report findings without editing anything.
    pub fn check_design(&self, params: &Value) -> Result<Value, CommandError> {
        let (view, effective) = self.design_check_inputs(params)?;
        let mut report = view.check_design(effective.values);
        report.sources = effective.sources;
        Ok(json!(report))
    }

    /// Refuse a design check's bad board address or rules without running the check, so the
    /// shared command path validates once and only the response builds the report.
    pub(crate) fn validate_design_check(&self, params: &Value) -> Result<(), CommandError> {
        self.design_check_inputs(params).map(drop)
    }

    /// The board view and rule overrides a design check reads, with the check's errors.
    fn design_check_inputs(&self, params: &Value) -> Result<(BoardView<'_>, super::rules::Effective), CommandError> {
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
        let mut overrides = params.clone();
        if overrides.is_null() {
            overrides = json!({});
        }
        if let Some(fields) = overrides.as_object_mut() {
            fields.remove("board");
        }
        Ok((view, view.resolve_rules(&overrides)?))
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
pub(crate) mod contour_tests;

#[cfg(test)]
pub(crate) mod short_tests;

mod job;
pub use job::{CheckJob, CheckProgress, CheckSource};
