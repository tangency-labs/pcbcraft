//! Authored single-layer copper-zone intent. Outlines never participate in conductive geometry.
use crate::{
    CommandError, Project, Session,
    nets::{ClassTarget, State},
    schematic::Point,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

/// Create zone intent and promote an eligible net in one transaction.
pub const ADD_ZONE: &str = "pcb.add_zone";
/// Replace the addressed zone properties while retaining its UUID.
pub const SET_ZONE: &str = "pcb.set_zone";
/// Remove a nonempty set of addressed zone outlines atomically.
pub const DELETE_ZONES: &str = "pcb.delete_zones";
/// Independent limits bound authored storage, picking and polygon validation.
pub const MAX_ZONES: usize = 256;
/// Maximum distinct vertices in one implicitly closed polygon.
pub const MAX_POINTS: usize = 256;
/// Board-wide vertex budget, independent of the number of outlines.
pub const MAX_TOTAL_POINTS: usize = 8192;
/// Bound saved island-removal intent to one square metre.
pub const MAX_ISLAND_AREA_NM2: u64 = 1_000_000_000_000_000_000;
/// Maximum polygon edges (each checked against all non-adjacent following edges) validated per
/// resumable work unit.
pub(crate) const ZONE_VALIDATION_EDGES_PER_UNIT: usize = 32;

#[cfg(test)]
thread_local! {
    static ZONE_EDGES_STEPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Return and reset the count of zone outline edges checked by [`ZoneValidationSteps::step`].
#[cfg(test)]
pub(crate) fn take_zone_edges_stepped() -> usize {
    ZONE_EDGES_STEPPED.with(|count| count.replace(0))
}

/// Future fill connection intent, without generating copper or spokes in this slice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    #[default]
    Thermal,
    Solid,
    None,
}

/// Fill settings belong to authored intent even before the fill implementation exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub priority: u16,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "clearance")]
    pub clearance_nm: Option<i64>,
    pub thermal_gap_nm: i64,
    pub spoke_width_nm: i64,
    pub connection: Connection,
    pub min_island_area_nm2: u64,
    /// Remove filled islands that touch no pad, via or track of the zone's net.
    #[serde(default)]
    pub remove_unconnected_islands: bool,
}
/// Omission inherits board clearance; explicit stored null is refused.
fn clearance<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<i64>, D::Error> {
    i64::deserialize(de).map(Some)
}
impl Default for Options {
    fn default() -> Self {
        Self {
            priority: 0,
            clearance_nm: None,
            thermal_gap_nm: 300_000,
            spoke_width_nm: 300_000,
            connection: Connection::Thermal,
            min_island_area_nm2: 0,
            remove_unconnected_islands: false,
        }
    }
}
impl Options {
    /// Refuse unbounded/unsupported fill intent before a transaction can publish it.
    pub fn validate(&self) -> Result<(), String> {
        let dimension = |n: i64| (super::track::MIN_WIDTH_NM..=super::track::MAX_WIDTH_NM).contains(&n);
        if self.priority > 1000
            || self.clearance_nm.is_some_and(|n| !dimension(n))
            || !dimension(self.thermal_gap_nm)
            || !dimension(self.spoke_width_nm)
            || self.min_island_area_nm2 > MAX_ISLAND_AREA_NM2
        {
            return Err("Zone settings need priority 0–1000, dimensions 10000–10000000 nm, and island area at most 1 m².".into());
        }
        Ok(())
    }
}

/// A closed, implicitly joined simple polygon attached to a persistent net and copper layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub id: Uuid,
    pub layer: Uuid,
    pub net: Uuid,
    pub points: Vec<Point>,
    pub options: Options,
}

/// Integer orientation avoids floating-point tolerance and overflow at coordinate limits.
fn orient(a: Point, b: Point, c: Point) -> i128 {
    (b.x_nm as i128 - a.x_nm as i128) * (c.y_nm as i128 - a.y_nm as i128) - (b.y_nm as i128 - a.y_nm as i128) * (c.x_nm as i128 - a.x_nm as i128)
}
/// Closed segments intersect on crossings, collinear overlap or endpoint touches.
fn intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    let inside = |p: Point, x: Point, y: Point| {
        (x.x_nm.min(y.x_nm)..=x.x_nm.max(y.x_nm)).contains(&p.x_nm) && (x.y_nm.min(y.y_nm)..=x.y_nm.max(y.y_nm)).contains(&p.y_nm)
    };
    let (ac, ad, ca, cb) = (orient(a, b, c), orient(a, b, d), orient(c, d, a), orient(c, d, b));
    (ac == 0 && inside(c, a, b))
        || (ad == 0 && inside(d, a, b))
        || (ca == 0 && inside(a, c, d))
        || (cb == 0 && inside(b, c, d))
        || (ac.signum() != ad.signum() && ca.signum() != cb.signum())
}
/// Verify that an outline has 3–256 distinct in-bounds vertices before checking edges.
fn validate_outline_vertices(points: &[Point]) -> Result<(), String> {
    if !(3..=MAX_POINTS).contains(&points.len())
        || points.iter().any(|p| !crate::wire::valid_point(p))
        || points.iter().map(|p| (p.x_nm, p.y_nm)).collect::<BTreeSet<_>>().len() != points.len()
    {
        return Err("A zone needs 3–256 distinct bounded vertices; closure is implicit.".into());
    }
    Ok(())
}
/// Check outline edges `start..end` against all non-adjacent following edges while accumulating
/// twice the signed polygon area.
fn validate_outline_edges(points: &[Point], start: usize, end: usize, area: &mut i128) -> Result<(), String> {
    let count = points.len();
    for i in start..end {
        let (a, b) = (points[i], points[(i + 1) % count]);
        *area += a.x_nm as i128 * b.y_nm as i128 - b.x_nm as i128 * a.y_nm as i128;
        for j in i + 1..count {
            if j == i + 1 || (i == 0 && j == count - 1) {
                continue;
            }
            if intersects(a, b, points[j], points[(j + 1) % count]) {
                return Err("Zone outlines cannot intersect or touch themselves.".into());
            }
        }
    }
    Ok(())
}
/// Require at least three distinct bounded vertices, nonzero area and no crossing/touching edges.
pub fn validate_outline(points: &[Point]) -> Result<(), String> {
    validate_outline_vertices(points)?;
    let mut area = 0i128;
    validate_outline_edges(points, 0, points.len(), &mut area)?;
    if area == 0 {
        return Err("A zone must enclose a nonzero area.".into());
    }
    Ok(())
}
/// Work units required to validate one zone: at least one unit, plus one per
/// [`ZONE_VALIDATION_EDGES_PER_UNIT`] outline edges.
fn zone_validation_units(zone: &Zone) -> usize {
    zone.points.len().div_ceil(ZONE_VALIDATION_EDGES_PER_UNIT).max(1)
}
/// Resumable validator for a board's authored copper zones.
///
/// Enforces the board-wide zone and total-vertex limits once at startup, then validates each
/// zone's identity, net registration, copper layer, options and outline in bounded batches of at
/// most [`ZONE_VALIDATION_EDGES_PER_UNIT`] polygon edges per work unit. Dropping the validator
/// mid-run cancels cleanly with no side effects.
pub(crate) struct ZoneValidationSteps {
    checked_limits: bool,
    zone_cursor: usize,
    edge_cursor: usize,
    area: i128,
    ids: BTreeSet<Uuid>,
}
impl ZoneValidationSteps {
    /// Start before checking board-wide limits and the first zone.
    pub(crate) fn new() -> Self {
        Self { checked_limits: false, zone_cursor: 0, edge_cursor: 0, area: 0, ids: BTreeSet::new() }
    }
    /// Work units required to validate every zone on the board (zero when no zones exist).
    pub(crate) fn total(project: &Project) -> usize {
        project.board.zones.iter().fold(0usize, |acc, zone| acc.saturating_add(zone_validation_units(zone)))
    }
    /// Advance at most `budget` zone-validation units, decrementing it by the units completed.
    /// Returns `Ok(true)` once every zone on the board has passed validation.
    pub(crate) fn step(&mut self, project: &Project, budget: &mut usize) -> Result<bool, String> {
        let zones = &project.board.zones;
        if zones.is_empty() {
            return Ok(true);
        }
        if !self.checked_limits {
            if *budget == 0 {
                return Ok(false);
            }
            if zones.len() > MAX_ZONES || zones.iter().fold(0usize, |acc, z| acc.saturating_add(z.points.len())) > MAX_TOTAL_POINTS {
                return Err("Zone resource limit exceeded.".into());
            }
            self.checked_limits = true;
        }
        while self.zone_cursor < zones.len() {
            if *budget == 0 {
                return Ok(false);
            }
            let z = &zones[self.zone_cursor];
            if self.edge_cursor == 0 {
                if z.id.is_nil() || !self.ids.insert(z.id) || project.net_registry.record(z.net).is_none() {
                    return Err("Zone has a duplicate/nil identity or unregistered net.".into());
                }
                if !project
                    .board
                    .setup
                    .as_ref()
                    .is_some_and(|s| s.layers().iter().any(|l| l.id == z.layer && l.purpose == super::setup::LayerPurpose::Copper))
                {
                    return Err("Zone needs a copper layer on its board.".into());
                }
                z.options.validate()?;
                validate_outline_vertices(&z.points)?;
                self.area = 0;
            }
            let end = self.edge_cursor.saturating_add(ZONE_VALIDATION_EDGES_PER_UNIT).min(z.points.len());
            #[cfg(test)]
            ZONE_EDGES_STEPPED.with(|count| count.set(count.get() + end.saturating_sub(self.edge_cursor)));
            validate_outline_edges(&z.points, self.edge_cursor, end, &mut self.area)?;
            *budget -= 1;
            self.edge_cursor = end;
            if self.edge_cursor == z.points.len() {
                if self.area == 0 {
                    return Err("A zone must enclose a nonzero area.".into());
                }
                self.zone_cursor += 1;
                self.edge_cursor = 0;
                self.area = 0;
            }
        }
        Ok(true)
    }
}
/// Validate all authored records against this physical board; inactive net identities remain inspectable.
pub(crate) fn validate(project: &Project) -> Result<(), String> {
    let mut budget = usize::MAX;
    if ZoneValidationSteps::new().step(project, &mut budget)? { Ok(()) } else { Err("zone validation did not complete".into()) }
}
impl Project {
    /// Report historical net state instead of silently redirecting a zone after a merge or split.
    pub fn zone_net_state(&self, net: Uuid) -> &'static str {
        match self.net_registry.record(net).map(|r| &r.state) {
            Some(State::Active {}) => "active",
            Some(State::Ambiguous { .. }) => "ambiguous",
            Some(State::Retired { .. }) => "retired",
            None => "missing",
        }
    }
    /// Inspect authored outlines plus current identity status; inspection never promotes nets.
    pub fn inspect_zones(&self) -> Value {
        json!(
            self.board
                .zones
                .iter()
                .map(|zone| {
                    let mut value = serde_json::to_value(zone).unwrap_or(Value::Null);
                    value["net_state"] = json!(self.zone_net_state(zone.net));
                    value
                })
                .collect::<Vec<_>>()
        )
    }
}
impl<'a> super::view::BoardView<'a> {
    /// Borrow stored outlines; no fill, copper membership or derived netlist is added.
    pub fn zones(self) -> &'a [Zone] {
        &self.project().board.zones
    }
    /// Inspect the board-owned current net choices without allocating identities.
    pub fn inspect_zone_nets(self) -> Result<Value, String> {
        self.project().inspect_net_registry()
    }
    /// Report unresolved identity status for the addressed board's zone inspector.
    pub fn zone_net_state(self, net: Uuid) -> &'static str {
        self.project().zone_net_state(net)
    }
}
impl Session {
    /// Add/edit/delete zone intent atomically. Net promotion shares the creation transaction;
    /// malformed paths, foreign owners, stale requests and resource limits publish nothing.
    pub(crate) fn edit_zone(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        original.resolve_board(params["board"].as_str())?;
        let mut next = original.clone();
        if command == DELETE_ZONES {
            let ids: Vec<Uuid> = serde_json::from_value(params["zones"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
            let unique: BTreeSet<_> = ids.iter().copied().collect();
            if ids.is_empty()
                || ids.len() > MAX_ZONES
                || unique.len() != ids.len()
                || unique.iter().any(|id| !next.board.zones.iter().any(|z| z.id == *id))
            {
                return Err(CommandError::InvalidParams("Select distinct existing zone IDs.".into()));
            }
            Arc::make_mut(&mut next.board).zones.retain(|z| !unique.contains(&z.id));
        } else {
            if params.get("net").is_some() && params.get("target").is_some() {
                return Err(CommandError::InvalidParams("Choose net or target, not both.".into()));
            }
            let net = if let Some(target) = params.get("target") {
                let (candidate, id) = next.with_promoted_net(&ClassTarget::parse(target)?)?;
                next = candidate;
                Some(id)
            } else {
                params
                    .get("net")
                    .map(|v| serde_json::from_value::<Uuid>(v.clone()).map_err(|e| CommandError::InvalidParams(e.to_string())))
                    .transpose()?
            };
            let zones = &mut Arc::make_mut(&mut next.board).zones;
            if command == ADD_ZONE {
                let z = Zone {
                    id: crate::fresh_id()?,
                    layer: serde_json::from_value(params["layer"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?,
                    net: net.ok_or_else(|| CommandError::InvalidParams("Choose a registered net or a current derived target.".into()))?,
                    points: serde_json::from_value(params["points"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?,
                    options: params
                        .get("options")
                        .map(|v| serde_json::from_value(v.clone()))
                        .transpose()
                        .map_err(|e| CommandError::InvalidParams(e.to_string()))?
                        .unwrap_or_default(),
                };
                zones.push(z);
            } else {
                let id: Uuid = serde_json::from_value(params["zone"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
                let zone = zones.iter_mut().find(|z| z.id == id).ok_or_else(|| CommandError::InvalidParams("Unknown zone.".into()))?;
                if let Some(net) = net {
                    zone.net = net;
                }
                if let Some(v) = params.get("layer") {
                    zone.layer = serde_json::from_value(v.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
                }
                if let Some(v) = params.get("points") {
                    zone.points = serde_json::from_value(v.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
                }
                if let Some(v) = params.get("options") {
                    zone.options = serde_json::from_value(v.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
                }
            }
            zones.sort_by_key(|z| z.id);
        }
        validate(&next).map_err(CommandError::InvalidParams)?;
        if next.board != original.board || next.net_registry != original.net_registry {
            self.change(next)?;
        }
        Ok(())
    }
}

/// Shared command schema for complete settings; runtime validation enforces the same bounds.
pub(crate) fn options_schema() -> Value {
    let dimension = json!({"type":"integer","minimum":super::track::MIN_WIDTH_NM,"maximum":super::track::MAX_WIDTH_NM});
    json!({"type":"object","additionalProperties":false,"required":["priority","thermal_gap_nm","spoke_width_nm","connection","min_island_area_nm2"],"properties":{"priority":{"type":"integer","minimum":0,"maximum":1000},"clearance_nm":dimension,"thermal_gap_nm":dimension,"spoke_width_nm":dimension,"connection":{"enum":["thermal","solid","none"]},"min_island_area_nm2":{"type":"integer","minimum":0,"maximum":MAX_ISLAND_AREA_NM2},"remove_unconnected_islands":{"type":"boolean"}}})
}

#[cfg(test)]
mod tests;
