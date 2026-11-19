//! Atomic route replacements, preserving source identities, intent and physical connections.
use super::*;
use crate::board::{
    clearance::{self, BoardGeometry, ClearanceQuery},
    drc,
    work::Source,
};
use std::sync::{Arc, Mutex, TryLockError};

/// One original segment replaced by a connected path of the same width and layer. Its first
/// resulting segment retains the original UUID; all other segments receive fresh UUIDs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    pub track: Uuid,
    pub points: Vec<Point>,
}

/// Reject malformed paths before constructing any candidate or allocating replacement IDs.
pub(crate) fn parse(value: &Value) -> Result<Vec<Replacement>, CommandError> {
    let replacements: Vec<Replacement> = serde_json::from_value(value.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
    let ids: BTreeSet<_> = replacements.iter().map(|r| r.track).collect();
    if replacements.len() > MAX_TRACKS || ids.len() != replacements.len() || ids.contains(&Uuid::nil()) {
        return Err(CommandError::InvalidParams("Name each replacement track once, within board limits.".into()));
    }
    for replacement in &replacements {
        if !(2..=MAX_PATH_POINTS).contains(&replacement.points.len())
            || replacement.points.iter().any(|p| !crate::wire::valid_point(p))
            || replacement.points.windows(2).any(|p| p[0] == p[1])
        {
            return Err(CommandError::InvalidParams("A replacement needs 2–256 bounded points with distinct neighbors.".into()));
        }
    }
    Ok(replacements)
}

/// Apply paths to a private candidate and retain every fragment's original source for validation.
/// Locked/unknown originals fail before their geometry is changed; source dimensions are retained.
pub(super) fn apply(tracks: &mut Vec<Track>, value: &Value, changed: &mut BTreeSet<Uuid>) -> Result<BTreeMap<Uuid, Uuid>, CommandError> {
    let replacements = parse(value)?;
    let positions: BTreeMap<_, _> = tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let count = tracks.len() + replacements.iter().map(|r| r.points.len() - 2).sum::<usize>();
    if count > MAX_TRACKS {
        return Err(CommandError::InvalidParams("The replacement paths exceed the board track limit.".into()));
    }
    for replacement in &replacements {
        let index = positions.get(&replacement.track).ok_or_else(|| CommandError::InvalidParams("Unknown replacement track.".into()))?;
        if tracks[*index].locked {
            return Err(CommandError::InvalidParams("Locked tracks cannot be replaced.".into()));
        }
    }
    let mut origins = BTreeMap::new();
    for replacement in replacements {
        let index = positions[&replacement.track];
        let original = tracks[index].clone();
        for (leg, points) in replacement.points.windows(2).enumerate() {
            let id = if leg == 0 { original.id } else { crate::fresh_id()? };
            let track = Track { id, start: points[0], end: points[1], ..original.clone() };
            if leg == 0 {
                tracks[index] = track;
            } else {
                tracks.push(track);
            }
            changed.insert(id);
            origins.insert(id, original.id);
        }
    }
    Ok(origins)
}

/// Resumable final validation against immutable source intent, independent of any solver.
/// The source query is prepared in bounded batches; object checks yield between source contacts
/// and changed contours. Dropping the validator never changes authored state or history.
pub(super) struct Validation {
    source: Source,
    preparation: Arc<Mutex<clearance::preparation::Preparation>>,
    final_geometry: Arc<BoardGeometry>,
    origins: BTreeMap<Uuid, Uuid>,
    changed: BTreeSet<Uuid>,
    origin: (Point, Uuid, i64),
    query: Option<Arc<ClearanceQuery>>,
    final_query: Option<ClearanceQuery>,
    final_positions: BTreeMap<String, usize>,
    final_origins: Vec<Option<usize>>,
    final_displaced: Vec<bool>,
    merge_same_net: bool,
    groups: BTreeMap<usize, usize>,
    cursor: usize,
    phase: u8,
}
impl Validation {
    /// Capture the final contact graph and replacement ownership; no result is accepted yet.
    pub(super) fn new(
        source: Source,
        preparation: Arc<Mutex<clearance::preparation::Preparation>>,
        final_geometry: Arc<BoardGeometry>,
        origins: BTreeMap<Uuid, Uuid>,
        changed: BTreeSet<Uuid>,
        params: &Value,
    ) -> Result<Self, CommandError> {
        let points: Vec<Point> = param(params, "points")?;
        let width_nm = match params.get("width_nm") {
            Some(_) => param(params, "width_nm")?,
            None => final_geometry
                .items
                .iter()
                .find(|item| Uuid::parse_str(&item.name).is_ok_and(|id| changed.contains(&id) && !origins.contains_key(&id)))
                .map(|item| (item.contour.radius * 2.) as i64)
                .ok_or_else(|| CommandError::Unavailable("Missing new route copper.".into()))?,
        };
        Ok(Self {
            preparation,
            source,
            final_positions: final_geometry.items.iter().enumerate().map(|(i, o)| (o.name.clone(), i)).collect(),
            final_geometry,
            final_origins: Vec::new(),
            final_displaced: Vec::new(),
            merge_same_net: params.get("merge_same_net").and_then(Value::as_bool).unwrap_or(false),
            origins,
            changed,
            origin: (points[0], param(params, "layer")?, width_nm),
            query: None,
            final_query: None,
            groups: BTreeMap::new(),
            cursor: 0,
            phase: 0,
        })
    }

    /// Preserve each source island as one final physical island, including interior pad and
    /// branch contacts. Then check changed contours with original membership/island identities,
    /// so a newly created connection cannot grant its own same-net or same-island exemption.
    pub(super) fn step(&mut self, budget: usize) -> Result<bool, CommandError> {
        if budget == 0 {
            return Ok(false);
        }
        if self.query.is_none() {
            self.query = match self.preparation.try_lock() {
                Ok(mut preparation) => preparation.step(budget)?,
                Err(TryLockError::WouldBlock) => return Ok(false),
                Err(TryLockError::Poisoned(_)) => return Err(CommandError::Unavailable("Replacement source preparation failed.".into())),
            };
            return Ok(false);
        }
        let source = self.query.as_ref().cloned().ok_or_else(|| CommandError::Unavailable("Missing replacement source query.".into()))?;
        if self.phase == 0 {
            for object in source.objects().iter().skip(self.cursor).take(budget.min(32)) {
                let &position = self
                    .final_positions
                    .get(&object.name)
                    .ok_or_else(|| CommandError::InvalidParams("A route replacement removed existing copper.".into()))?;
                let island = self.final_geometry.items[position].island;
                if self.groups.insert(object.island, island).is_some_and(|old| old != island) {
                    return Err(CommandError::InvalidParams("A route replacement would disconnect existing copper.".into()));
                }
            }
            self.cursor = self.cursor.saturating_add(budget.min(32));
            if self.cursor < source.objects().len() {
                return Ok(false);
            }
            self.final_query = Some(self.with_source_intent(&source)?);
            self.cursor = 0;
            self.phase = 1;
            return Ok(false);
        }
        let query = self.final_query.as_ref().ok_or_else(|| CommandError::Unavailable("Missing replacement validation query.".into()))?;
        let view = self.source.snapshot.board_view(self.source.target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
        let outline = view.outline_edges();
        for (index, object) in query.objects().iter().enumerate().skip(self.cursor).take(budget.min(8)) {
            let changed = Uuid::parse_str(&object.name).is_ok_and(|id| self.changed.contains(&id));
            if !changed {
                continue;
            }
            let result = query.probe(&object.contour, &object.layers, Some(index))?;
            for decision in &result.decisions {
                let other = &query.objects()[decision.second];
                let same_net = object.intended_nets.len() == 1 && object.intended_nets == other.intended_nets;
                let same_island = object.island == other.island;
                let original_contact = match (self.final_origins[index], self.final_origins[decision.second]) {
                    (Some(a), Some(b)) => {
                        a == b
                            || (source.objects()[a].layers.iter().any(|l| source.objects()[b].layers.contains(l))
                                && source.objects()[a].contour.touches(&source.objects()[b].contour))
                    }
                    _ => false,
                };
                let exempt = if !self.merge_same_net && (self.final_displaced[index] || self.final_displaced[decision.second]) {
                    original_contact
                } else {
                    same_net || same_island
                };
                if decision.conflicting_nets || (!exempt && decision.gap_nm < decision.clearance_nm as f64) {
                    return Err(CommandError::InvalidParams("The compound route violates copper clearance or intended net membership.".into()));
                }
            }
            let baseline = self.source.effective.values;
            let rules = result.probe_rules.as_ref().map_or(baseline, |p| p.hard_values(baseline));
            if object.contour.radius * 2. < rules.min_track_width_nm as f64 {
                return Err(CommandError::InvalidParams("The compound route violates a minimum track width.".into()));
            }
            if !outline.polygons.is_empty() {
                let (gap, crosses) = drc::edge_gap(&outline.edges, &object.contour);
                if !drc::contains(&outline.polygons, &object.contour) || crosses || gap < rules.edge_clearance_nm as f64 {
                    return Err(CommandError::InvalidParams("The compound route violates the Board Outline clearance.".into()));
                }
            }
        }
        self.cursor = self.cursor.saturating_add(budget.min(8));
        Ok(self.cursor >= query.objects().len())
    }

    /// Final contours retain their original island/membership; only genuinely new route
    /// fragments inherit the original pickup contact, never the contacts of a speculative path.
    fn with_source_intent(&mut self, source: &ClearanceQuery) -> Result<ClearanceQuery, CommandError> {
        let by_name: BTreeMap<_, _> = source.objects().iter().enumerate().map(|(i, o)| (o.name.as_str(), (i, o))).collect();
        let (at, layer, width) = self.origin;
        let pickup = PadContour::segment(at, at, width)?;
        let contacts: Vec<_> = source.candidates(&pickup, &[layer])?.into_iter().filter(|i| source.objects()[*i].contour.touches(&pickup)).collect();
        let nets: Vec<_> =
            contacts.iter().flat_map(|i| source.objects()[*i].intended_nets.iter().copied()).collect::<BTreeSet<_>>().into_iter().collect();
        let island = contacts.first().map_or(usize::MAX, |i| source.objects()[*i].island);
        let mut items = self.final_geometry.items.clone();
        for item in &mut items {
            let id = Uuid::parse_str(&item.name).ok();
            let original = id.and_then(|id| self.origins.get(&id)).map(Uuid::to_string);
            let inherited = by_name.get(original.as_deref().unwrap_or(&item.name));
            self.final_displaced.push(id.is_some_and(|id| self.origins.contains_key(&id)));
            if let Some((index, original)) = inherited {
                self.final_origins.push(Some(*index));
                item.intended_nets = original.intended_nets.clone();
                item.island = original.island;
            } else {
                self.final_origins.push(contacts.first().copied());
                item.intended_nets = nets.clone();
                item.island = island;
            }
        }
        source.with_objects(items)
    }
}

/// Bounded additive command schema; an empty list requests strict compound-route validation.
pub(crate) fn schema() -> Value {
    json!({"type":"array","maxItems":MAX_TRACKS,"items":{"type":"object","properties":{
        "track":{"type":"string","format":"uuid"},"points":path_schema()
    },"required":["track","points"],"additionalProperties":false}})
}

#[cfg(test)]
mod tests;
