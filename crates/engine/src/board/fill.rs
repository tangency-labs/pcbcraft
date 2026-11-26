//! Source-owned copper fill derived from authored zones and existing physical copper contours.
//! Nothing in this module is encoded in a native project document.
use super::{
    clearance::{BoardGeometry, Object},
    geometry::Contour,
    track::Copper,
    view::{BoardTarget, BoardView},
    zones::{Connection, Zone},
};
use crate::schematic::Point;
use pcbcraft_geometry::planar::{DisplayMesh, Operation, Regions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

mod cache;
mod cuts;
mod templates;
pub mod worker;
pub(crate) use cache::Cache;
pub use cache::FillJob;
use cuts::{Cuts, near};
use templates::{Keepout, Memo, Thermal};

/// Numerical reserve outside requested copper/edge clearances. The offset kernel's 0.1 nm
/// loop tolerance and derived floating coordinates never authorize rounding into an obstacle.
pub const CLEARANCE_RESERVE_NM: i64 = 2;
/// Recalculate current board copper without changing authored intent or document history.
pub const REFILL_ZONES: &str = "pcb.refill_zones";
/// Remove derived zone copper until an explicit pour, retaining all authored zone intent.
pub const UNFILL_ZONES: &str = "pcb.unfill_zones";
/// Inspect the current source's pending/ready/unavailable fill without scheduling work.
pub const INSPECT_FILL: &str = "pcb.inspect_zone_fill";

/// Typed fill diagnostics retain their meaning independently of human-readable messages.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum ProblemKind {
    NetUnavailable,
    RemovedIslandPads,
    Unavailable,
    Clearance,
    BoardEdge,
}

/// An explicit zone problem; unavailable copper is never an empty successful fill.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Problem {
    pub kind: ProblemKind,
    pub zone: Uuid,
    pub message: String,
    pub at: Point,
    pub items: Vec<String>,
}

/// One retained material island with exact geometry and display-only triangles.
#[derive(Clone, Debug)]
pub struct Island {
    pub region: Regions,
    pub mesh: DisplayMesh,
    pub contacts: Vec<usize>,
}

/// Why a zone policy removed an island from the fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalReason {
    /// Smaller than the zone's minimum island area.
    MinimumArea,
    /// Touches no pad, via or track of the zone's net.
    Unconnected,
}

/// One island a zone policy removed, reported so the removal can be reviewed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemovedIsland {
    pub reason: RemovalReason,
    pub area_nm2: f64,
    /// Minimum x, minimum y, maximum x and maximum y.
    pub bounds_nm: [f64; 4],
}

/// Completed fill of one authored zone; every island retains that zone's UUID/net/layer.
#[derive(Clone, Debug)]
pub struct ZoneFill {
    pub id: Uuid,
    pub net: Uuid,
    pub layer: Uuid,
    pub islands: Vec<Island>,
    /// The number of removed islands, equal to the length of `removed`.
    pub dropped: usize,
    pub removed: Vec<RemovedIsland>,
    pub clearance_nm: i64,
}

/// One complete derived result for one immutable board input. Snapshot authority lives in
/// the job/cache owner, so the result cannot keep a project/cache reference cycle alive.
#[derive(Clone, Debug)]
pub struct Fill {
    pub board: BoardTarget,
    pub zones: Vec<ZoneFill>,
    pub problems: Vec<Problem>,
    pub copper: Copper,
    pub(crate) members: Vec<Vec<usize>>,
}

/// Prepared raw physical objects retain their existing contour vocabulary and exact logical
/// membership. Primitive geometry is converted lazily once for each operation's required margin.
struct Obstacle {
    object: Object,
    net: Option<Uuid>,
    pad: bool,
    clearance_nm: i64,
}

/// Convert the existing physical core plus radius, without approximating pad shapes a second
/// time or rounding odd copper widths. All generation and contact checks call this adapter.
fn region(contour: &Contour, margin: i64) -> Result<Regions, String> {
    Regions::rounded_core(&contour.core.map(|p| [p.x_nm, p.y_nm]), contour.radius + margin as f64).map_err(|e| e.to_string())
}

/// Clip without translating a kernel refusal into empty material.
fn clip(a: &Regions, b: &Regions, operation: Operation) -> Result<Regions, String> {
    a.boolean(b, operation).map_err(|e| e.to_string())
}

/// Resolve raw intended memberships against current persistent terminals. An accepted net
/// containing missing or competing terminal identities stays unknown, rather than being
/// silently assigned the first name or UUID found on its copper island.
fn obstacles(view: BoardView<'_>, geometry: &BoardGeometry) -> Result<Vec<Obstacle>, String> {
    let project = view.project();
    let current = project.persistent_terminal_nets().map_err(|e| e.to_string())?;
    let mut membership = BTreeMap::<usize, Option<Uuid>>::new();
    if let Some(baseline) = &project.board.baseline {
        for ((component, pin), index) in baseline.terminal_net_index() {
            let id = current.get(&(component, pin.to_owned())).copied();
            membership
                .entry(index)
                .and_modify(|previous| {
                    if *previous != id {
                        *previous = None
                    }
                })
                .or_insert(id);
        }
    } else {
        let graph = project.electrical_connectivity();
        for ((component, pin), id) in &current {
            let Some(index) = project.component_terminal(*component, pin).and_then(|t| graph.net_index_for(t)) else { continue };
            membership
                .entry(index)
                .and_modify(|previous| {
                    if *previous != Some(*id) {
                        *previous = None
                    }
                })
                .or_insert(Some(*id));
        }
    }
    for (id, index) in project.board_intent_memberships().ids {
        membership
            .entry(index)
            .and_modify(|previous| {
                if *previous != Some(id) {
                    *previous = None;
                }
            })
            .or_insert(Some(id));
    }
    let class_rules = super::class_rules::NetRules::new(view, view.effective_rules()).map_err(|e| e.to_string())?;
    let pads: BTreeSet<_> = geometry.lands.iter().map(|land| format!("{}:{}", land.key.0, land.key.1)).collect();
    Ok(geometry
        .items
        .iter()
        .cloned()
        .map(|object| {
            let net = match object.intended_nets.as_slice() {
                [index] => membership.get(index).copied().flatten(),
                _ => None,
            };
            let pad = pads.contains(&object.name);
            let clearance_nm =
                class_rules.as_ref().map_or(view.effective_rules().values.clearance_nm, |rules| rules.clearance(&object.intended_nets));
            Obstacle { object, net, pad, clearance_nm }
        })
        .collect())
}

/// Build material inside all supported closed Board Outline paths. Missing closure is an
/// unavailable fill; an authored zone outline is not used as a substitute board boundary.
fn board_region(view: BoardView<'_>) -> Result<Regions, String> {
    let layers: BTreeSet<_> = view.layers().into_iter().filter(|l| l.purpose == super::setup::LayerPurpose::Outline).map(|l| l.id).collect();
    let mut paths = Vec::new();
    for path in view.drawings().into_iter().filter(|d| d.closed && layers.contains(&d.layer)) {
        let polygon = Regions::polygon(&path.points.iter().map(|p| [p.x_nm, p.y_nm]).collect::<Vec<_>>()).map_err(|e| e.to_string())?;
        paths.push(polygon);
    }
    let result = Regions::union_all(&paths).map_err(|e| e.to_string())?;
    if result.is_empty() {
        return Err("Zone fill needs a closed Board Outline.".into());
    }
    Ok(result)
}

/// Generate one zone against an immutable board and already completed higher-priority fills.
/// Equal-priority foreign outlines receive mutual clearance, avoiding a UUID-order winner.
fn zone_fill(
    view: BoardView<'_>,
    zone: &Zone,
    board: &Regions,
    objects: &[Obstacle],
    fill: &mut Fill,
    memo: &Memo,
    canceled: Option<&AtomicBool>,
) -> Result<ZoneFill, String> {
    let rules = view.effective_rules().values;
    let clearance = zone.options.clearance_nm.unwrap_or(rules.clearance_nm).max(rules.clearance_nm) + CLEARANCE_RESERVE_NM;
    let outline = Regions::polygon(&zone.points.iter().map(|p| [p.x_nm, p.y_nm]).collect::<Vec<_>>()).map_err(|e| e.to_string())?;
    let inset = board.offset(-(rules.edge_clearance_nm + CLEARANCE_RESERVE_NM)).map_err(|e| e.to_string())?;
    let base = clip(&outline, &inset, Operation::Intersection)?;
    let bounds = base.bounds_nm();
    let mut cuts = Vec::new();
    for other in objects.iter().filter(|o| o.object.layers.contains(&zone.layer) && o.net != Some(zone.net)) {
        check_cancel(canceled)?;
        let margin = clearance.max(other.clearance_nm + CLEARANCE_RESERVE_NM);
        if bounds.is_some_and(|b| near(b, other.object.contour.bounds(), margin as f64 + cuts::BOUND_RESERVE_NM)) {
            cuts.push(Keepout::physical(&other.object.contour, margin, memo)?);
        }
    }
    for other in fill.zones.iter().filter(|other| other.layer == zone.layer && other.net != zone.net) {
        for island in &other.islands {
            check_cancel(canceled)?;
            let margin = clearance.max(other.clearance_nm);
            if let (Some(base), Some(foreign)) = (bounds, island.region.bounds_nm())
                && !near(base, foreign, margin as f64 + cuts::BOUND_RESERVE_NM)
            {
                continue;
            }
            let expanded = island.region.offset(margin).map_err(|e| e.to_string())?;
            cuts::add_plane(&mut cuts, expanded, canceled)?;
        }
    }
    for other in view.zones().iter().filter(|other| {
        other.id != zone.id
            && other.layer == zone.layer
            && other.net != zone.net
            && other.options.priority == zone.options.priority
            && view.project().zone_net_state(other.net) == "active"
    }) {
        check_cancel(canceled)?;
        let polygon = Regions::polygon(&other.points.iter().map(|p| [p.x_nm, p.y_nm]).collect::<Vec<_>>()).map_err(|e| e.to_string())?;
        let pair_clearance = clearance.max(other.options.clearance_nm.unwrap_or(rules.clearance_nm) + CLEARANCE_RESERVE_NM);
        cuts.push(Keepout::plane(polygon.offset(pair_clearance).map_err(|e| e.to_string())?));
    }
    check_cancel(canceled)?;
    let cuts = if zone.options.connection == Connection::Solid {
        Cuts::new(cuts.into_iter().map(|cut| cut.region).collect())
    } else {
        let gap =
            zone.options.thermal_gap_nm.max(if zone.options.connection == Connection::None { rules.clearance_nm } else { 0 }) + CLEARANCE_RESERVE_NM;
        let pads: Vec<_> = objects
            .iter()
            .filter(|o| o.net == Some(zone.net) && o.object.layers.contains(&zone.layer) && o.pad)
            .filter(|o| {
                let b = o.object.contour.bounds();
                let reach = (b[2] - b[0]).max(b[3] - b[1]).ceil() + (gap + zone.options.spoke_width_nm) as f64;
                bounds.is_some_and(|stock| near(stock, b, reach))
            })
            .collect();
        if zone.options.connection == Connection::Thermal {
            let mut thermals = Vec::with_capacity(pads.len());
            for pad in pads {
                check_cancel(canceled)?;
                thermals.push(Thermal::new(&pad.object, zone.options.spoke_width_nm, zone.options.thermal_gap_nm, memo)?);
            }
            templates::thermal_cuts(&base, cuts, thermals, memo, canceled)?
        } else {
            for pad in pads {
                check_cancel(canceled)?;
                cuts.push(Keepout::physical(&pad.object.contour, gap, memo)?);
            }
            Cuts::new(cuts.into_iter().map(|cut| cut.region).collect())
        }
    };
    let material = cuts.subtract(base, canceled)?;
    let mut result =
        ZoneFill { id: zone.id, net: zone.net, layer: zone.layer, islands: Vec::new(), dropped: 0, removed: Vec::new(), clearance_nm: clearance };
    for island in material.islands() {
        check_cancel(canceled)?;
        let mut contacts = Vec::new();
        let bounds = island.bounds_nm().ok_or("Retained island has no bounds.")?;
        // Foreign and unknown copper were removed with positive clearance before the
        // thermal step. Every accepted spoke is clipped to that same safe material.
        for (index, object) in objects.iter().enumerate().filter(|(_, o)| {
            o.net == Some(zone.net) && o.object.layers.contains(&zone.layer) && near(bounds, o.object.contour.bounds(), cuts::BOUND_RESERVE_NM)
        }) {
            check_cancel(canceled)?;
            let center = [object.object.at.x_nm, object.object.at.y_nm];
            // The resolved center is interior to each physical pad, via or track contour.
            // For boundary/partial contacts keep the exact positive-area intersection test.
            if island.contains(center).map_err(|e| e.to_string())? == Some(true)
                || (island.may_intersect_box(object.object.contour.bounds()).map_err(|e| e.to_string())?
                    && !clip(&island, &region(&object.object.contour, 0)?, Operation::Intersection)?.is_empty())
            {
                contacts.push(index);
            }
        }
        let area_nm2 = island.area().map_err(|e| e.to_string())?.value_nm2;
        if area_nm2 < zone.options.min_island_area_nm2 as f64 {
            result.dropped += 1;
            result.removed.push(RemovedIsland { reason: RemovalReason::MinimumArea, area_nm2, bounds_nm: bounds });
            let items: Vec<_> = contacts.iter().filter(|i| objects[**i].pad).map(|i| objects[*i].object.name.clone()).collect();
            if !items.is_empty() {
                fill.problems.push(Problem {
                    kind: ProblemKind::RemovedIslandPads,
                    zone: zone.id,
                    message: "Minimum-area island removal disconnected pads.".into(),
                    at: zone.points[0],
                    items,
                });
            }
        } else if zone.options.remove_unconnected_islands && contacts.is_empty() {
            // No same-net pad, via or track touches this island, so it cannot carry the net.
            result.dropped += 1;
            result.removed.push(RemovedIsland { reason: RemovalReason::Unconnected, area_nm2, bounds_nm: bounds });
        } else {
            let mesh = island.display_mesh(5_000).map_err(|e| e.to_string())?;
            result.islands.push(Island { region: island, mesh, contacts });
        }
    }
    Ok(result)
}

/// Compute from one immutable snapshot. The scheduled job calls this outside the UI thread;
/// any geometric failure leaves the entire result unavailable and all authored data unchanged.
pub fn compute(view: BoardView<'_>) -> Result<Arc<Fill>, String> {
    compute_cancellable(view, None, &mut |_| {})
}

/// Worker calculation samples cancellation before preparation and between indivisible kernel
/// operations. Aborted calculations return no partial physical connectivity or display mesh.
fn compute_cancellable(view: BoardView<'_>, canceled: Option<&AtomicBool>, report: &mut dyn FnMut(usize)) -> Result<Arc<Fill>, String> {
    check_cancel(canceled)?;
    view.require_linear_copper()?;
    let lands = super::track::lands(view);
    let (copper, members) = super::track::islands(&lands, view.tracks(), view.vias());
    let geometry = BoardGeometry::from_islands(view, lands, copper.clone(), members.clone());
    let objects = obstacles(view, &geometry)?;
    let mut fill = Fill { board: view.target, zones: Vec::new(), problems: Vec::new(), copper, members };
    if view.zones().is_empty() {
        join_contacts(&geometry, &mut fill, canceled)?;
        return Ok(Arc::new(fill));
    }
    let board = board_region(view)?;
    let memo = view.project().zone_geometry_memo();
    let mut zones: Vec<_> = view.zones().iter().collect();
    zones.sort_by_key(|z| (std::cmp::Reverse(z.options.priority), z.id));
    for (index, zone) in zones.into_iter().enumerate() {
        check_cancel(canceled)?;
        if view.project().zone_net_state(zone.net) != "active" {
            fill.problems.push(Problem {
                kind: ProblemKind::NetUnavailable,
                zone: zone.id,
                message: format!("Zone net is {}.", view.project().zone_net_state(zone.net)),
                at: zone.points[0],
                items: Vec::new(),
            });
        } else {
            let result = zone_fill(view, zone, &board, &objects, &mut fill, &memo, canceled)?;
            fill.zones.push(result);
        }
        report(index + 1);
    }
    join_contacts(&geometry, &mut fill, canceled)?;
    Ok(Arc::new(fill))
}

/// Merge physical roots through actual retained fill regions, including overlaps between
/// zones. Each region is its own node; sharing a zone UUID alone never joins disconnected islands.
fn join_contacts(geometry: &BoardGeometry, fill: &mut Fill, canceled: Option<&AtomicBool>) -> Result<(), String> {
    let islands: Vec<_> = fill.zones.iter().flat_map(|z| z.islands.iter().map(move |i| (z, i))).collect();
    let base = fill.members.len();
    let mut parent: Vec<_> = (0..base + islands.len()).collect();
    /// Path compression belongs to this disposable result, never the model or saved copper.
    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }
    /// Join only roots supported by represented physical material.
    fn join(parent: &mut [usize], a: usize, b: usize) {
        let (a, b) = (find(parent, a), find(parent, b));
        parent[b] = a;
    }
    // Preserve existing direct-pad physical contacts as well as legacy routed roots. The
    // fill's node membership can then serve the same short checker without a second contact pass.
    for nodes in &geometry.members {
        if let Some(first) = nodes.first() {
            for other in &nodes[1..] {
                join(&mut parent, geometry.items[*first].island, geometry.items[*other].island);
            }
        }
    }
    for (index, (_, island)) in islands.iter().enumerate() {
        for object in &island.contacts {
            join(&mut parent, base + index, geometry.items[*object].island);
        }
    }
    for i in 0..islands.len() {
        for j in i + 1..islands.len() {
            check_cancel(canceled)?;
            if islands[i].0.layer == islands[j].0.layer && !clip(&islands[i].1.region, &islands[j].1.region, Operation::Intersection)?.is_empty() {
                join(&mut parent, base + i, base + j);
            }
        }
    }
    let roots: Vec<_> = (0..base).map(|i| find(&mut parent, i)).collect();
    let mut canonical = BTreeMap::new();
    for root in &roots {
        let next = canonical.len();
        canonical.entry(*root).or_insert(next);
    }
    for value in fill.copper.pads.values_mut().chain(fill.copper.tracks.values_mut()).chain(fill.copper.vias.values_mut()) {
        *value = canonical[&roots[*value]];
    }
    let mut merged = vec![Vec::new(); canonical.len()];
    for (index, nodes) in std::mem::take(&mut fill.members).into_iter().enumerate() {
        merged[canonical[&roots[index]]].extend(nodes);
    }
    for nodes in &mut merged {
        nodes.sort_unstable();
    }
    fill.members = merged;
    Ok(())
}

/// Cancellation is an unavailable result, never a successfully empty pour.
fn check_cancel(canceled: Option<&AtomicBool>) -> Result<(), String> {
    if canceled.is_some_and(|flag| flag.load(Ordering::Acquire)) { Err("Zone fill canceled.".into()) } else { Ok(()) }
}

#[cfg(test)]
mod tests;

/// Check the current analytic fill against diagnostic thresholds and source contours. A
/// stricter check never regenerates the material with different geometry or effective rules.
pub(super) fn diagnostics(view: BoardView<'_>, geometry: &BoardGeometry, rules: super::rules::Values) -> Vec<Problem> {
    if view.zones().is_empty() {
        return Vec::new();
    }
    diagnostics_with_fill(view, geometry, rules).1
}

/// Either dilation direction establishes the same pair clearance. Fall back
/// to offsetting the other member when a concave clipped loop cannot be offset.
fn tighter_zone_violation(a: &Regions, b: &Regions, clearance_nm: i64) -> Result<bool, String> {
    if let (Some(x), Some(y)) = (a.bounds_nm(), b.bounds_nm())
        && !near(x, y, clearance_nm as f64 + cuts::BOUND_RESERVE_NM)
    {
        return Ok(false);
    }
    let intersects = |within: &Regions, offsetting: &Regions| {
        let grown = offsetting.offset(clearance_nm).map_err(|e| format!("Could not expand a plane for clearance checking: {e}"))?;
        clip(within, &grown, Operation::Intersection)
            .map(|overlap| !overlap.is_empty())
            .map_err(|e| format!("Could not compare the expanded planes: {e}"))
    };
    intersects(b, a).or_else(|first| intersects(a, b).map_err(|second| format!("{first} / {second}")))
}

/// Keep analytic findings and the corresponding physical connectivity in one captured result.
/// A failed fill yields an unavailable-zone finding and no conductive fallback material.
pub(super) fn diagnostics_with_fill(view: BoardView<'_>, geometry: &BoardGeometry, rules: super::rules::Values) -> (Option<Arc<Fill>>, Vec<Problem>) {
    if view.zones().is_empty() {
        return (view.filled_zones().and_then(Result::ok), Vec::new());
    }
    let mut captured = None;
    let inspect = |captured: &mut Option<Arc<Fill>>| -> Result<Vec<Problem>, String> {
        let filled = view.fill_for_check()?;
        *captured = Some(filled.clone());
        let mut problems = filled.problems.clone();
        // A successful, source-matched pour has already applied the authoring clearance
        // and edge inset, including the numerical reserve. Repeat the geometric checks only
        // where a request asks for more than the corresponding cuts guaranteed.
        let authored_rules = view.effective_rules().values;
        let objects = if rules.clearance_nm > authored_rules.clearance_nm { obstacles(view, geometry)? } else { Vec::new() };
        let inset = (rules.edge_clearance_nm > authored_rules.edge_clearance_nm)
            .then(|| board_region(view)?.offset(-rules.edge_clearance_nm).map_err(|e| e.to_string()))
            .transpose()?;
        for (index, zone) in filled.zones.iter().enumerate() {
            let authored = view.zones().iter().find(|z| z.id == zone.id).ok_or("Missing authored fill zone")?;
            for island in &zone.islands {
                if let Some(inset) = &inset
                    && !clip(&island.region, inset, Operation::Difference)?.is_empty()
                {
                    problems.push(Problem {
                        kind: ProblemKind::BoardEdge,
                        zone: zone.id,
                        message: "Filled copper violates the board-edge clearance.".into(),
                        at: authored.points[0],
                        items: Vec::new(),
                    });
                }
                // Check in the same deterministic priority order used to build the cuts.
                // Expanding the earlier plane also avoids offsetting a more complex low-priority
                // island that was already carved around that plane.
                for other in filled.zones.iter().skip(index + 1).filter(|other| other.net != zone.net && other.layer == zone.layer) {
                    let guaranteed = zone.clearance_nm.max(other.clearance_nm);
                    if rules.clearance_nm <= guaranteed - CLEARANCE_RESERVE_NM {
                        continue;
                    }
                    for foreign in &other.islands {
                        if tighter_zone_violation(&island.region, &foreign.region, rules.clearance_nm)? {
                            problems.push(Problem {
                                kind: ProblemKind::Clearance,
                                zone: zone.id,
                                message: "Filled copper violates another zone's clearance.".into(),
                                at: authored.points[0],
                                items: vec![other.id.to_string()],
                            });
                        }
                    }
                }
                let bounds = island.region.bounds_nm().ok_or("Retained island has no bounds.")?;
                for other in objects.iter().filter(|o| o.net != Some(zone.net) && o.object.layers.contains(&zone.layer)) {
                    let guaranteed = other.clearance_nm.max(zone.clearance_nm - CLEARANCE_RESERVE_NM);
                    if rules.clearance_nm > guaranteed
                        && near(bounds, other.object.contour.bounds(), rules.clearance_nm as f64 + cuts::BOUND_RESERVE_NM)
                        && !clip(&island.region, &region(&other.object.contour, rules.clearance_nm)?, Operation::Intersection)
                            .map_err(|e| format!("Could not check clearance to {}: {e}", other.object.name))?
                            .is_empty()
                    {
                        problems.push(Problem {
                            kind: ProblemKind::Clearance,
                            zone: zone.id,
                            message: "Filled copper violates another net's clearance.".into(),
                            at: other.object.at,
                            items: vec![other.object.name.clone()],
                        });
                    }
                }
            }
        }
        Ok(problems)
    };
    let problems = match inspect(&mut captured) {
        Ok(problems) => problems,
        Err(message) => unavailable(view, message),
    };
    (captured, problems)
}

/// Associate a valid-source calculation failure with authored zone intent, without inventing copper.
pub(super) fn unavailable(view: BoardView<'_>, message: String) -> Vec<Problem> {
    view.zones()
        .first()
        .map(|zone| vec![Problem { kind: ProblemKind::Unavailable, zone: zone.id, message, at: zone.points[0], items: Vec::new() }])
        .unwrap_or_default()
}
