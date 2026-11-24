//! A bounded design rules check over one board snapshot.
//!
//! The check reads the board without editing it. It reports copper clearance between different
//! nets, conflicting intended nets within a copper island, minimum track width, via annular ring,
//! unrouted connections, the board outline (missing, copper outside it or inside a cut-out,
//! copper too close to its edge) and overlapping footprint courtyards, decided from their
//! actual supported shapes. Rule values resolve saved board defaults and bounded request-local
//! overrides through the shared resolver. Results are deterministic, so the same board and rules
//! give the same report. Analytic zone fill shares connectivity and spacing checks.
//! Manufacturing sign-off also needs native circular route, hole-to-hole spacing, silkscreen
//! and solder mask checks.
use super::{
    clearance::{BoardGeometry, Object as Item},
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
    /// An authored zone refers to a retired or unresolved persistent net.
    ZoneNetUnavailable,
    /// Minimum-area removal detached pads from a filled island.
    ZoneDisconnected,
    /// Current zone geometry could not be calculated or validated completely.
    ZoneFillUnavailable,
    /// Applicable hard requirements contradict, so the affected checking result is incomplete.
    NetClassConflict,
    /// Saved via intent no longer resolves to one active net on this board.
    ViaNetUnresolved,
    /// Assigned policy could not be captured, so checking cannot establish a complete result.
    UnsupportedNetClasses,
    /// Circular copper prevents complete checking; this finding replaces partial results.
    UnsupportedGeometry,
    /// One physical copper island joins copper with distinct intended nets, independently of
    /// the clearance rule, including when the required clearance is zero.
    Short,
    Clearance,
    TrackWidth,
    ViaDiameter,
    ViaDrill,
    AnnularRing,
    Unconnected,
    NoOutline,
    /// Two or more closed Board Outline drawings cross, touch or share an edge, so their
    /// nesting cannot be resolved; they are treated as a union while disjoint contours still
    /// nest by parity.
    AmbiguousOutline,
    OutsideBoard,
    EdgeClearance,
    CourtyardOverlap,
}

/// Intended membership participating in a short. `net` is snapshot-local accepted or logical
/// membership, extended by explicit via-only intent, never a persisted ID.
/// Names are display text only; component-terminal keys identify the involved logical objects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShortNet {
    /// Index in this checked snapshot's membership projection, never a saved net ID.
    pub net: usize,
    /// Display label for this membership; equal labels do not imply equal nets.
    pub name: String,
    /// Involved logical terminals, sorted and deduplicated by component identity and pin text.
    pub terminals: Vec<TerminalKey>,
}

/// One finding: what, where (a representative point in board nanometers) and which items.
/// Items are footprint IDs (`footprint-N`), pads (`footprint-N:pad`), mechanical
/// openings (`footprint-N:npth:index`, 1-based in the pinned definition), or track/via UUIDs.
/// A copper item or non-plated hole inside a board cut-out also lists the cut-out's Board
/// Outline drawing UUID.
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
    /// Snapshot-local copper addresses refer to deduplicated class/origin/conflict reports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_rules: Option<super::class_rules::Context>,
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
    ambiguous_outlines: std::collections::BTreeSet<Vec<String>>,
    total: usize,
}

impl Findings {
    /// Retain bounded detail without losing totals; board-level outline findings are reported once.
    fn push(&mut self, violation: Violation) {
        if violation.kind == Kind::NoOutline && self.counts.contains_key(&Kind::NoOutline) {
            return;
        }
        if violation.kind == Kind::AmbiguousOutline && !self.ambiguous_outlines.insert(violation.items.clone()) {
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
        Report { rules, sources, class_rules: None, violations: self.listed.into_values().collect(), counts: self.counts, total: self.total }
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

/// Keep the existing copper diagnostics and identify mechanical openings in both DRC paths.
fn clearance_message(a: &Item, b: &Item, distance: f64, clearance_nm: i64) -> String {
    if a.mechanical || b.mechanical {
        return if distance <= 0. {
            "Non-plated hole touches copper".into()
        } else {
            format!("Non-plated hole to copper clearance {} is below the {} rule", mm(distance), mm(clearance_nm as f64))
        };
    }
    let nets = match (&a.net, &b.net) {
        (Some(x), Some(y)) => format!(" between nets {x} and {y}"),
        _ => String::new(),
    };
    if distance <= 0. {
        format!("Copper touches{nets}")
    } else {
        format!("Clearance {} is below the {} rule{nets}", mm(distance), mm(clearance_nm as f64))
    }
}

/// Format a length for messages in millimeters, independent of the viewer's display units.
fn mm(nm: f64) -> String {
    format!("{:.3} mm", nm / 1_000_000.)
}

/// Capture exact per-object authority once for either checker. Unavailable policy becomes an
/// explicit incomplete finding; only contradictions in dimensions the checker uses block it.
fn capture_class_rules(
    view: BoardView<'_>,
    geometry: &BoardGeometry,
    effective: &super::rules::Effective,
    found: &mut Findings,
) -> Option<super::class_rules::Context> {
    let captured =
        super::class_rules::NetRules::new(view, effective.clone()).and_then(|rules| rules.map(|rules| rules.capture(view, geometry)).transpose());
    let context = match captured {
        Ok(Some(context)) if !context.policies.is_empty() => context,
        Ok(_) => return None,
        Err(error) => {
            found.push(Violation {
                kind: Kind::UnsupportedNetClasses,
                message: error.to_string(),
                at: Point { x_nm: 0, y_nm: 0 },
                items: Vec::new(),
                nets: Vec::new(),
            });
            return None;
        }
    };
    if context
        .policies
        .iter()
        .all(|policy| policy.resolution.conflicts.iter().all(|conflict| conflict.properties.iter().any(|property| property.is_preference())))
    {
        return Some(context);
    }
    use crate::net_classes::Property::*;
    let tracks: std::collections::BTreeSet<_> = view.tracks().iter().map(|track| track.id.to_string()).collect();
    let vias: std::collections::BTreeSet<_> = view.vias().iter().map(|via| via.id.to_string()).collect();
    let mut conflicts = BTreeMap::<(usize, String), Vec<String>>::new();
    for (name, &index) in &context.objects {
        let mut needed = vec![ClearanceNm, EdgeClearanceNm];
        if tracks.contains(name) {
            needed.push(MinTrackWidthNm);
        }
        if vias.contains(name) {
            needed.extend([MinViaDiameterNm, MaxViaDiameterNm, MinViaDrillNm, MaxViaDrillNm, MinAnnularRingNm]);
        }
        if let Err(error) = context.policies[index].require(&needed) {
            conflicts.entry((index, error.to_string())).or_default().push(name.clone());
        }
    }
    for ((_, message), items) in conflicts {
        let at = geometry.items.iter().find(|item| items.contains(&item.name)).map_or(Point { x_nm: 0, y_nm: 0 }, |item| item.at);
        found.push(Violation { kind: Kind::NetClassConflict, message, at, items, nets: Vec::new() });
    }
    Some(context)
}

impl BoardView<'_> {
    /// Native and isolated browser zone diagnostics share the same stable finding mapping.
    fn add_zone_problems(problems: Vec<super::fill::Problem>, found: &mut Findings) {
        use super::fill::ProblemKind;
        for problem in problems {
            let kind = match problem.kind {
                ProblemKind::NetUnavailable => Kind::ZoneNetUnavailable,
                ProblemKind::RemovedIslandPads => Kind::ZoneDisconnected,
                ProblemKind::Unavailable => Kind::ZoneFillUnavailable,
                ProblemKind::Clearance => Kind::Clearance,
                ProblemKind::BoardEdge => Kind::EdgeClearance,
            };
            let mut items = vec![problem.zone.to_string()];
            items.extend(problem.items);
            found.push(Violation { kind, message: problem.message, at: problem.at, items, nets: Vec::new() });
        }
    }

    /// Check this board against `rules` and return every finding, sorted by kind and position.
    pub fn check_design(self, rules: Rules) -> Report {
        let mut effective = self.effective_rules();
        let supplied = json!(rules);
        let base = json!(effective.values);
        for name in super::rules::LIMIT_FIELDS.iter().chain(super::rules::PREFERENCE_FIELDS) {
            if supplied[*name] != base[*name] {
                effective.sources.insert((*name).into(), super::rules::Source::Request);
            }
        }
        effective.values = rules;
        self.check_design_resolved(effective)
    }

    /// Sync and incremental checks retain identical request provenance and object policy.
    fn check_design_resolved(self, effective: super::rules::Effective) -> Report {
        let rules = effective.values;
        let mut found = Findings::default();
        if let Err(message) = self.require_linear_copper() {
            found.push(Violation {
                kind: Kind::UnsupportedGeometry,
                message,
                at: Point { x_nm: 0, y_nm: 0 },
                items: self
                    .arc_tracks()
                    .iter()
                    .map(|arc| arc.id.to_string())
                    .chain(self.teardrops().iter().map(|record| record.id.to_string()))
                    .collect(),
                nets: Vec::new(),
            });
            return found.finish(rules, effective.sources);
        }
        let geometry = BoardGeometry::new(self);
        let class_rules = capture_class_rules(self, &geometry, &effective, &mut found);
        let items = &geometry.items;
        let (fill, problems) = super::fill::diagnostics_with_fill(self, &geometry, rules);
        Self::add_zone_problems(problems, &mut found);
        self.check_dimensions(rules, class_rules.as_ref(), &mut found);
        self.check_slot_rings(&geometry.lands, rules, class_rules.as_ref(), &mut found);

        let members = fill.as_ref().map_or(&geometry.members, |fill| &fill.members);
        self.check_shorts(&geometry.lands, members, items, &mut found);

        // Physical island short diagnostics remain independent of the strict clearance rule.
        for decision in geometry.clearance_pairs(rules.clearance_nm, class_rules.as_ref()) {
            if !decision.clearance_violation {
                continue;
            }
            if let Some(i) = decision.first {
                let (a, b) = (&items[i], &items[decision.second]);
                let message = clearance_message(a, b, decision.gap_nm, decision.clearance_nm);
                let mut names = vec![a.name.clone(), b.name.clone()];
                names.sort();
                found.push(Violation { kind: Kind::Clearance, message, at: midpoint(a.at, b.at), items: names, nets: Vec::new() });
            }
        }

        if let Ok(ratsnest) = self.ratsnest() {
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

        self.check_outline(items, rules, class_rules.as_ref(), &mut found);
        self.check_courtyards(&mut found);

        let mut report = found.finish(rules, effective.sources);
        report.class_rules = class_rules;
        report
    }

    /// Check authored track/via dimensions without preparing connectivity or changing state.
    fn check_dimensions(self, rules: Rules, context: Option<&super::class_rules::Context>, found: &mut Findings) {
        for track in self.tracks() {
            let rules = context.map_or(rules, |context| context.limits_for(&track.id.to_string(), rules));
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
            let rules = context.map_or(rules, |context| context.limits_for(&via.id.to_string(), rules));
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

    /// Report slot ring minima on the same resolved geometry used for copper preparation.
    fn check_slot_rings(self, lands: &[Land], rules: Rules, context: Option<&super::class_rules::Context>, found: &mut Findings) {
        for land in lands {
            let Some(slot) = land.plated_slot else { continue };
            let key = format!("{}:{}", land.key.0, land.key.1);
            let minimum = context.map_or(rules, |context| context.limits_for(&key, rules)).min_annular_ring_nm;
            let ring = slot.annular_ring_nm(&land.contour());
            if ring < minimum as f64 {
                found.push(Violation {
                    kind: Kind::AnnularRing,
                    message: format!("Plated slot annular ring is {}, below the {} minimum", mm(ring), mm(minimum as f64)),
                    at: land.center,
                    items: vec![key],
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
        let graph = baseline.is_none().then(|| project.electrical_connectivity());
        let intent = project.board_intent_memberships();
        let intrinsic = super::via_nets::node_membership(project, lands, self.tracks().len(), self.vias(), &intent);
        for via in self.vias() {
            if let Some(address) = via.net_assignment.filter(|a| project.via_net_state(*a) != "active") {
                found.push(Violation {
                    kind: Kind::ViaNetUnresolved,
                    message: format!("Via net assignment is {}; review or clear its target.", project.via_net_state(address)),
                    at: via.at,
                    items: vec![via.id.to_string()],
                    nets: Vec::new(),
                });
            }
        }
        let memberships: Vec<_> = lands
            .iter()
            .map(|land| {
                let pad = super::Pad::parse(&land.key.0, &land.key.1)?;
                let (component, terminal) = project.board.component_pin_for(pad)?;
                Some(TerminalKey { component, terminal: terminal.to_owned() })
            })
            .collect();
        for island in members {
            let assigned = island
                .iter()
                .filter_map(|n| n.checked_sub(lands.len() + self.tracks().len()))
                .filter_map(|i| self.vias().get(i))
                .find(|v| v.net_assignment.is_some_and(|a| project.via_net_state(a) == "active"));
            if let Some(via) = assigned
                && !island.iter().any(|n| *n < lands.len())
            {
                found.push(Violation {
                    kind: Kind::Unconnected,
                    message: "Assigned via island has no physical pad connection.".into(),
                    at: via.at,
                    items: island.iter().map(|n| items[*n].name.clone()).collect(),
                    nets: Vec::new(),
                });
            }
            let mut nets = BTreeMap::<usize, Vec<TerminalKey>>::new();
            for &node in island {
                if let Some(net) = intrinsic[node] {
                    let terminals = nets.entry(net).or_default();
                    if let Some(Some(terminal)) = memberships.get(node) {
                        terminals.push(terminal.clone());
                    }
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
                    let name = intent
                        .names
                        .get(&net)
                        .cloned()
                        .or_else(|| baseline.and_then(|baseline| baseline.net_name(net)))
                        .or_else(|| graph.as_ref().map(|graph| graph.net_name(net)));
                    ShortNet { net, name: name.unwrap_or_default(), terminals }
                })
                .collect();
            let names: Vec<_> = nets.iter().map(|net| format!("{} (net {})", net.name, net.net + 1)).collect();
            let at = island.first().map(|node| items[*node].at);
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

    /// Require a closed Board Outline, every copper item on the board, and copper at least the
    /// edge clearance from every outline segment. Closed contours whose boundaries meet are
    /// grouped into one union component and reported as `ambiguous_outline`, while disjoint
    /// contours continue to nest by even-odd parity so a cut-out away from an overlap stays a
    /// cut-out and names its drawing. Penetration remains a finding at zero required clearance.
    /// Open outline paths still count as edges.
    fn check_outline(self, items: &[Item], rules: Rules, context: Option<&super::class_rules::Context>, found: &mut Findings) {
        let outline = self.outline_edges();
        let Outline { drawn, polygons, ambiguous, edges, .. } = &outline;
        if polygons.is_empty() {
            if !items.is_empty() || *drawn {
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
        for (at, drawings) in ambiguous {
            found.push(Violation {
                kind: Kind::AmbiguousOutline,
                message: "Closed Board Outline drawings meet; their nesting is ambiguous and they are treated as one union".into(),
                at: *at,
                items: drawings.iter().map(Uuid::to_string).collect(),
                nets: Vec::new(),
            });
        }
        for item in items {
            let subject = if item.mechanical { "Non-plated hole" } else { "Copper" };
            let rules = context.map_or(rules, |context| context.limits_for(&item.name, rules));
            if let Some(around) = outline.off_board(&item.contour) {
                let mut names = vec![item.name.clone()];
                let message = if around.is_empty() {
                    format!("{subject} lies outside the Board Outline")
                } else {
                    names.extend(outline.blocking_edge(&item.contour, true).map(|(drawing, _, _)| drawing.to_string()));
                    format!("{subject} lies inside a board cut-out")
                };
                found.push(Violation { kind: Kind::OutsideBoard, message, at: item.at, items: names, nets: Vec::new() });
                continue;
            }
            let (distance, crosses) = edge_gap(edges, &item.contour);
            if crosses || distance < rules.edge_clearance_nm as f64 {
                found.push(Violation {
                    kind: Kind::EdgeClearance,
                    message: if crosses {
                        format!("{subject} crosses the Board Outline")
                    } else {
                        format!("{subject} is {} from the board edge, below the {} rule", mm(distance.max(0.)), mm(rules.edge_clearance_nm as f64))
                    },
                    at: item.at,
                    items: vec![item.name.clone()],
                    nets: Vec::new(),
                });
            }
        }
    }

    /// Report same-side footprints whose courtyard interiors overlap, through the checker shared
    /// with the resumable job. Touching courtyards pass; unsupported geometry is compared by its
    /// bounds and reported as conservative. Pairs follow placement order.
    fn check_courtyards(self, found: &mut Findings) {
        let courtyards: Vec<_> = self.footprints().filter_map(|footprint| courtyard::Courtyard::capture(&footprint)).collect();
        for (i, a) in courtyards.iter().enumerate() {
            for b in &courtyards[i + 1..] {
                if let Some(violation) = a.overlap(b) {
                    found.push(violation);
                }
            }
        }
    }
}

/// The board's Board Outline drawings: closed polygons that bound the board, and every
/// edge, closed or open, that copper must keep its edge clearance from. Closed polygons whose
/// boundaries cross, touch or share an edge are grouped into one union component, while each
/// disjoint closed polygon forms its own component. A point is on the board when it lies
/// inside an odd number of components: an outer contour (or union of meeting outer contours)
/// is board, a disjoint contour nested inside it is a cut-out, and one nested inside a
/// cut-out is board again, as for the 3D board substrate. Each multi-polygon component is
/// recorded in `ambiguous` so DRC reports that nesting among those meeting drawings could not
/// be resolved.
pub(super) struct Outline {
    /// Whether any outline drawing exists, even one that is not closed.
    pub(super) drawn: bool,
    pub(super) polygons: Vec<Vec<Point>>,
    /// The drawing of each closed polygon, in the order of `polygons`.
    drawings: Vec<Uuid>,
    /// Connected component index of each closed polygon under boundary meeting.
    component: Vec<usize>,
    /// Number of connected components in `component`.
    component_count: usize,
    /// Multi-polygon components whose boundaries meet: marker point and sorted drawing UUIDs.
    ambiguous: Vec<(Point, Vec<Uuid>)>,
    pub(super) edges: Vec<OutlineEdge>,
}

pub(super) struct OutlineEdge {
    drawing: Uuid,
    closed: bool,
    start: Point,
    end: Point,
}

impl OutlineEdge {
    /// Measure the shared gap and distinguish interior penetration from boundary contact.
    fn measure(&self, contour: &Contour) -> (f64, bool) {
        let gap = contour.gap(&Contour::capsule(self.start, self.end, 0.));
        let crosses = gap < 0. || (gap == 0. && contour.penetrates_segment(self.start, self.end));
        (gap, crosses)
    }
}

/// The edges of a closed polygon, including its closing edge.
fn polygon_edges(polygon: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    polygon.iter().copied().zip(polygon.iter().copied().cycle().skip(1))
}

/// Closed bounds `[min x, min y, max x, max y]` of a point set.
fn point_bounds(points: &[Point]) -> [i64; 4] {
    points.iter().fold([i64::MAX, i64::MAX, i64::MIN, i64::MIN], |b, p| [b[0].min(p.x_nm), b[1].min(p.y_nm), b[2].max(p.x_nm), b[3].max(p.y_nm)])
}

/// Whether two closed bounds share at least one point.
fn bounds_meet(a: [i64; 4], b: [i64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// Whether two closed polygons cross, touch or share any boundary point, exactly in integer
/// nanometers. Only edges that reach the other polygon's bounds are compared, so contours far
/// apart, or a small cut-out well inside the outer contour, cost little.
fn polygons_meet(a: &[Point], b: &[Point]) -> bool {
    let (a_bounds, b_bounds) = (point_bounds(a), point_bounds(b));
    if !bounds_meet(a_bounds, b_bounds) {
        return false;
    }
    let b_edges: Vec<_> = edges_near(b, a_bounds).collect();
    edges_near(a, b_bounds).any(|(p, q)| b_edges.iter().any(|&(r, s)| courtyard::segments_meet(p, q, r, s)))
}

/// The edges of a closed polygon whose bounds meet the given bounds.
fn edges_near(polygon: &[Point], bounds: [i64; 4]) -> impl Iterator<Item = (Point, Point)> + '_ {
    polygon_edges(polygon).filter(move |&(p, q)| bounds_meet(point_bounds(&[p, q]), bounds))
}

/// The copper's smallest distance to any outline edge, and whether an edge enters its interior.
/// A contained edge can have gap zero even though it enters a pad's copper.
pub(super) fn edge_gap(edges: &[OutlineEdge], contour: &Contour) -> (f64, bool) {
    edges.iter().fold((f64::INFINITY, false), |(best, crosses), edge| {
        let (gap, penetrates) = edge.measure(contour);
        (best.min(gap), crosses || penetrates)
    })
}

impl Outline {
    /// Closed outlines constrain copper placement and enforce the effective edge clearance.
    pub(super) fn allows_copper(&self, contour: &Contour, clearance_nm: i64) -> bool {
        if self.polygons.is_empty() {
            return true;
        }
        let (gap, crosses) = edge_gap(&self.edges, contour);
        !crosses && gap >= clearance_nm as f64 && self.contains(contour)
    }

    /// Whether every core anchor of the copper lies on the board. A point is on the board when
    /// it lies inside an odd number of boundary-meeting components, so a disjoint contour
    /// nested inside the board stays a cut-out even when other contours overlap each other.
    pub(super) fn contains(&self, contour: &Contour) -> bool {
        contour.core.iter().all(|point| self.on_board(*point))
    }

    /// Even-odd parity of the boundary-meeting components containing `point`.
    fn on_board(&self, point: Point) -> bool {
        let mut seen = vec![false; self.component_count];
        let mut around = 0usize;
        for (polygon, &comp) in self.polygons.iter().zip(&self.component) {
            if !seen[comp] && in_polygon(point, polygon) {
                seen[comp] = true;
                around += 1;
            }
        }
        around % 2 == 1
    }

    /// The closed drawings around the copper's first core anchor that is off the board, or
    /// `None` when every anchor is on the board. The list is empty when that anchor lies
    /// outside every closed contour; otherwise it lies inside a cut-out of the listed drawings.
    pub(super) fn off_board(&self, contour: &Contour) -> Option<Vec<Uuid>> {
        let point = contour.core.iter().copied().find(|point| !self.on_board(*point))?;
        Some(self.polygons.iter().zip(&self.drawings).filter(|(polygon, _)| in_polygon(point, polygon)).map(|(_, drawing)| *drawing).collect())
    }

    /// The innermost closed drawing enclosing the copper's first off-board anchor when that
    /// anchor lies inside a cut-out, or `None` when every anchor is on the board or the first
    /// off-board anchor lies outside every closed contour. Nesting depth counts distinct
    /// enclosing components so meeting outer contours do not outrank a nested cut-out.
    fn innermost_cut_out(&self, contour: &Contour) -> Option<Uuid> {
        let point = contour.core.iter().copied().find(|point| !self.on_board(*point))?;
        let enclosing: Vec<(usize, &[Point], Uuid)> = self
            .polygons
            .iter()
            .zip(&self.drawings)
            .zip(&self.component)
            .filter(|((polygon, _), _)| in_polygon(point, polygon))
            .map(|((polygon, drawing), &comp)| (comp, polygon.as_slice(), *drawing))
            .collect();
        enclosing
            .iter()
            .map(|&(comp, poly, drawing)| {
                let mut seen = vec![false; self.component_count];
                let mut depth = 0usize;
                for &(other_comp, outer, _) in &enclosing {
                    if other_comp != comp && !seen[other_comp] && poly.first().is_some_and(|&v| in_polygon(v, outer)) {
                        seen[other_comp] = true;
                        depth += 1;
                    }
                }
                (depth, drawing)
            })
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
            .map(|(_, drawing)| drawing)
    }

    /// Address the blocking drawing and the gap measured to its edge. For off-board copper use
    /// the nearest closed path (open paths cannot enclose copper); inside a cut-out restrict
    /// candidate edges to the innermost closed contour enclosing the off-board anchor, so copper
    /// that crosses both the cut-out and an outer contour still names the cut-out. Otherwise
    /// prefer crossing edges over merely near or touching ones. Break distance ties by drawing
    /// UUID.
    pub(super) fn blocking_edge(&self, contour: &Contour, outside: bool) -> Option<(Uuid, f64, bool)> {
        let target = if outside { self.innermost_cut_out(contour) } else { None };
        self.edges
            .iter()
            .filter(|edge| !outside || edge.closed)
            .filter(|edge| target.is_none_or(|drawing| edge.drawing == drawing))
            .map(|edge| {
                let (gap, crosses) = edge.measure(contour);
                (edge.drawing, gap, crosses)
            })
            .min_by(|a, b| {
                let crossing_priority = if outside { std::cmp::Ordering::Equal } else { b.2.cmp(&a.2) };
                crossing_priority.then_with(|| a.1.total_cmp(&b.1)).then_with(|| a.0.cmp(&b.0))
            })
    }
}

impl BoardView<'_> {
    /// Collect Board Outline drawings shared by DRC and route planning. Open paths contribute
    /// edges only; closed drawings contribute a polygon and their closing edge.
    pub(super) fn outline_edges(self) -> Outline {
        let layers = self.layers();
        let outline: Vec<_> = self
            .drawings()
            .into_iter()
            .filter(|drawing| layers.iter().any(|layer| layer.id == drawing.layer && layer.purpose == LayerPurpose::Outline))
            .collect();
        let polygons: Vec<Vec<Point>> = outline.iter().filter(|drawing| drawing.closed).map(|drawing| drawing.points.clone()).collect();
        let drawings: Vec<Uuid> = outline.iter().filter(|drawing| drawing.closed).map(|drawing| drawing.id).collect();
        let edges = outline
            .iter()
            .flat_map(|drawing| {
                let points = &drawing.points;
                let count = if drawing.closed { points.len() } else { points.len().saturating_sub(1) };
                (0..count).map(move |i| OutlineEdge {
                    drawing: drawing.id,
                    closed: drawing.closed,
                    start: points[i],
                    end: points[(i + 1) % points.len()],
                })
            })
            .collect();
        let mut component = vec![usize::MAX; polygons.len()];
        let mut component_count = 0usize;
        let mut ambiguous = Vec::new();
        for start in 0..polygons.len() {
            if component[start] != usize::MAX {
                continue;
            }
            component[start] = component_count;
            let mut stack = vec![start];
            let mut members = Vec::new();
            while let Some(i) = stack.pop() {
                members.push(i);
                for (j, other) in polygons.iter().enumerate() {
                    if component[j] == usize::MAX && polygons_meet(&polygons[i], other) {
                        component[j] = component_count;
                        stack.push(j);
                    }
                }
            }
            if members.len() > 1 {
                let mut pairs: Vec<(Uuid, Point)> =
                    members.into_iter().map(|idx| (drawings[idx], polygons[idx].first().copied().unwrap_or(Point { x_nm: 0, y_nm: 0 }))).collect();
                pairs.sort_by_key(|(id, _)| *id);
                if let Some(&(_, at)) = pairs.first() {
                    let ids: Vec<Uuid> = pairs.into_iter().map(|(id, _)| id).collect();
                    ambiguous.push((at, ids));
                }
            }
            component_count += 1;
        }
        ambiguous.sort_by(|a, b| a.1.cmp(&b.1));
        Outline { drawn: !outline.is_empty(), polygons, drawings, component, component_count, ambiguous, edges }
    }
}

impl Session {
    /// Validate the board address and rules, then report findings without editing anything.
    pub fn check_design(&self, params: &Value) -> Result<Value, CommandError> {
        let (view, effective) = self.design_check_inputs(params)?;
        Ok(json!(view.check_design_resolved(effective)))
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
        view.require_linear_copper().map_err(CommandError::Unavailable)?;
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

mod courtyard;

mod job;
pub use job::{CheckJob, CheckProgress, CheckSource};
