//! Atomic route replacements, preserving source identities, intent and physical connections.
use super::*;
use crate::board::{
    clearance::{self, BoardGeometry, ClearanceQuery, Object},
    work::Source,
};
use std::sync::{Arc, Mutex, TryLockError};

/// One original segment replaced by a connected path of the same width and layer. Its first
/// resulting segment retains the original UUID; all other segments receive fresh UUIDs.
/// Ordinary degree-two joins may move together; final validation independently preserves
/// trace continuity, free ends, branches and fixed pad/via/lock contacts.
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

/// Displacements submitted with a new route or derived by the connected drag planner.
/// Keeping the accepted geometry typed lets preview and validation share the candidate.
#[derive(Default)]
pub(super) struct Changes {
    pub tracks: Vec<Replacement>,
    pub vias: Vec<ViaMove>,
}

impl Changes {
    /// An explicit empty replacement list still requests full compound clearance validation.
    pub(super) fn from_params(params: &Value) -> Result<Option<Self>, CommandError> {
        let paths = params.get("replacements").map(parse).transpose()?;
        let vias = params.get("via_moves").map(via_moves).transpose()?.unwrap_or_default();
        Ok((paths.is_some() || !vias.is_empty()).then(|| Self { tracks: paths.unwrap_or_default(), vias }))
    }
}

/// Apply paths to a private candidate and retain every fragment's original source for validation.
/// Locked/unknown originals fail before their geometry is changed; source dimensions are retained.
pub(super) fn apply(
    tracks: &mut Vec<Track>,
    replacements: &[Replacement],
    changed: &mut BTreeSet<Uuid>,
) -> Result<BTreeMap<Uuid, Uuid>, CommandError> {
    let positions: BTreeMap<_, _> = tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    let count = tracks.len() + replacements.iter().map(|r| r.points.len() - 2).sum::<usize>();
    if count > MAX_TRACKS {
        return Err(CommandError::InvalidParams("The replacement paths exceed the board track limit.".into()));
    }
    for replacement in replacements {
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
    origin: Option<(Point, Uuid, i64)>,
    joints: BTreeSet<Uuid>,
    query: Option<Arc<ClearanceQuery>>,
    final_query: Option<ClearanceQuery>,
    final_positions: BTreeMap<String, usize>,
    split_fragments: BTreeMap<String, Vec<usize>>,
    final_origins: Vec<Option<usize>>,
    final_displaced: Vec<bool>,
    merge_same_net: bool,
    groups: BTreeMap<usize, usize>,
    cursor: usize,
    phase: u8,
    chain_job: Option<super::chains::Validation>,
    chains: Option<Arc<super::chains::Chains>>,
    chain_paths: Vec<Replacement>,
    chain_checked: bool,
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
        changes: &Changes,
    ) -> Result<Self, CommandError> {
        let view = source.snapshot.board_view(source.target).ok_or_else(|| CommandError::Unavailable("Missing board.".into()))?;
        let moves = &changes.vias;
        let dragged: Vec<Ends> = params.get("tracks").map(|_| param(params, "tracks")).transpose()?.unwrap_or_default();
        let mut joints: BTreeSet<_> = dragged.iter().map(|row| row.track).chain(moves.iter().map(|m| m.via)).collect();
        if !joints.is_empty() && !view.zones().is_empty() {
            return Err(CommandError::Unavailable("Moving routing joints on boards with zones is not supported.".into()));
        }
        let mut chain_paths = changes.tracks.clone();
        let mut centers = BTreeMap::new();
        for movement in moves {
            let index =
                view.vias().binary_search_by_key(&movement.via, |v| v.id).map_err(|_| CommandError::InvalidParams("Unknown source via.".into()))?;
            let old = view.vias()[index].at;
            centers.insert((old.x_nm, old.y_nm), movement.at);
        }
        if !centers.is_empty() {
            let paths: BTreeMap<_, _> = chain_paths.iter().map(|r| (r.track, &r.points)).collect();
            for track in view.tracks() {
                let ends = [track.start, track.end].map(|p| centers.get(&(p.x_nm, p.y_nm)).copied().unwrap_or(p));
                if ends != [track.start, track.end] {
                    if paths.get(&track.id).is_none_or(|points| points.as_slice() != ends) {
                        return Err(CommandError::InvalidParams("All tracks ending on a moved via must follow its center.".into()));
                    }
                    joints.insert(track.id);
                }
            }
        }
        chain_paths.retain(|r| !joints.contains(&r.track));
        let origin = if params.get("points").is_some() {
            let points: Vec<Point> = param(params, "points")?;
            let width = match params.get("width_nm") {
                Some(_) => param(params, "width_nm")?,
                None => final_geometry
                    .items
                    .iter()
                    .find(|item| {
                        Uuid::parse_str(&item.name).is_ok_and(|id| changed.contains(&id) && !origins.contains_key(&id) && !joints.contains(&id))
                    })
                    .map(|item| (item.contour.radius * 2.) as i64)
                    .ok_or_else(|| CommandError::Unavailable("Missing new route copper.".into()))?,
            };
            Some((points[0], param(params, "layer")?, width))
        } else {
            None
        };
        let chain_checked = chain_paths.is_empty();
        let final_positions: BTreeMap<_, _> = final_geometry.items.iter().enumerate().map(|(i, o)| (o.name.clone(), i)).collect();
        let mut split_fragments: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        if !joints.is_empty() {
            for (fragment, original) in &origins {
                if fragment != original
                    && let Some(&index) = final_positions.get(&fragment.to_string())
                {
                    split_fragments.entry(original.to_string()).or_default().push(index);
                }
            }
        }
        Ok(Self {
            chain_paths,
            chain_checked,
            chain_job: None,
            chains: None,
            preparation,
            source,
            final_positions,
            split_fragments,
            final_geometry,
            final_origins: Vec::new(),
            final_displaced: Vec::new(),
            merge_same_net: params.get("merge_same_net").and_then(Value::as_bool).unwrap_or(false),
            origins,
            changed,
            origin,
            joints,
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
        if !self.chain_checked {
            if self.chain_job.is_none() {
                self.chain_job = Some(super::chains::Validation::new(source.clone(), std::mem::take(&mut self.chain_paths), self.merge_same_net));
            }
            if let Some(job) = &mut self.chain_job
                && let Some(result) = job.step(budget.min(16))
            {
                self.chains = Some(result?);
                self.chain_job = None;
                self.chain_checked = true;
            }
            return Ok(false);
        }
        if self.phase == 0 {
            for object in source.objects().iter().skip(self.cursor).take(budget.min(32)) {
                let &position = self
                    .final_positions
                    .get(&object.name)
                    .ok_or_else(|| CommandError::InvalidParams("A route replacement removed existing copper.".into()))?;
                if !self.joints.is_empty() && Uuid::parse_str(&object.name).is_ok_and(|id| self.joints.contains(&id)) {
                    for i in source.candidates(&object.contour, &object.layers)? {
                        let other = &source.objects()[i];
                        if other.name == object.name || !object.contour.touches(&other.contour) {
                            continue;
                        }
                        let items = &self.final_geometry.items;
                        let kept = self
                            .final_positions
                            .get(&other.name)
                            .into_iter()
                            .chain(self.split_fragments.get(&other.name).into_iter().flatten())
                            .any(|&j| self.retains_joint(object, other, &items[position], &items[j], 0.));
                        if !kept {
                            return Err(CommandError::InvalidParams("Moving this joint would lose a protected copper contact.".into()));
                        }
                    }
                }
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
                let joint_contact = decision.touches
                    && !self.joints.is_empty()
                    && [index, decision.second].iter().any(|&i| {
                        self.final_origins[i]
                            .is_some_and(|original| Uuid::parse_str(&source.objects()[original].name).is_ok_and(|id| self.joints.contains(&id)))
                    });
                let needs_clearance = decision.gap_nm < decision.clearance_nm as f64 || joint_contact;
                let exempt = if !self.merge_same_net && (self.final_displaced[index] || self.final_displaced[decision.second]) {
                    needs_clearance && self.preserves_local_contact(&source, query, index, decision.second, decision.clearance_nm as f64)
                } else {
                    same_net || same_island
                };
                if decision.conflicting_nets || (other.mechanical && (decision.clearance_violation || joint_contact)) || (!exempt && needs_clearance)
                {
                    return Err(CommandError::InvalidParams("The compound route violates copper clearance or intended net membership.".into()));
                }
            }
            let baseline = self.source.effective.values;
            let rules = result.probe_rules.as_ref().map_or(baseline, |p| p.hard_values(baseline));
            if let Ok(id) = Uuid::parse_str(&object.name)
                && self.joints.contains(&id)
                && !joint_sizes(view, id, &object.contour, rules)
            {
                return Err(CommandError::InvalidParams("The moved copper violates its dimensional rules.".into()));
            }
            if object.contour.track_centerline().is_some() && object.contour.radius * 2. < rules.min_track_width_nm as f64 {
                return Err(CommandError::InvalidParams("The compound route violates a minimum track width.".into()));
            }
            if !outline.allows_copper(&object.contour, rules.edge_clearance_nm) {
                return Err(CommandError::InvalidParams("The compound route violates the Board Outline clearance.".into()));
            }
        }
        self.cursor = self.cursor.saturating_add(budget.min(8));
        Ok(self.cursor >= query.objects().len())
    }

    /// Within a source run, the complete stroke validator checks bends and
    /// returns. Across runs, allow a bend to depart from retained source contact
    /// or the incoming pickup, while preserving spacing beyond the junction.
    fn preserves_local_contact(&self, source: &ClearanceQuery, final_query: &ClearanceQuery, a: usize, b: usize, clearance: f64) -> bool {
        let (shape_a, shape_b) = (&final_query.objects()[a].contour, &final_query.objects()[b].contour);
        let (old_a, old_b) = match (self.final_origins[a], self.final_origins[b]) {
            (Some(old_a), Some(old_b)) => (old_a, old_b),
            (Some(original), None) => return self.preserves_pickup(source, original, shape_a, shape_b, clearance),
            (None, Some(original)) => return self.preserves_pickup(source, original, shape_b, shape_a, clearance),
            (None, None) => return false,
        };
        if !self.joints.is_empty()
            && [old_a, old_b].iter().any(|i| Uuid::parse_str(&source.objects()[*i].name).is_ok_and(|id| self.joints.contains(&id)))
        {
            return self.retains_joint(
                &source.objects()[old_a],
                &source.objects()[old_b],
                &final_query.objects()[a],
                &final_query.objects()[b],
                clearance,
            );
        }
        let Some(chains) = &self.chains else { return false };
        if chains.same_run(old_a, old_b) {
            return true;
        }
        if !chains.contact(old_a, old_b) {
            return false;
        }
        let (original_a, original_b) = (&source.objects()[old_a].contour, &source.objects()[old_b].contour);
        if !original_a.touches(original_b) {
            return false;
        }
        (within_source(shape_a, original_a) && within_source(shape_b, original_b))
            || (within_contact(shape_a, shape_b, original_a, original_b, clearance)
                && within_contact(shape_b, shape_a, original_b, original_a, clearance))
    }

    /// Check the same source contact during topology and final clearance validation. A split
    /// support retains the chain's fixed contact, so a fragment has no whole-track station.
    fn retains_joint(&self, old_a: &Object, old_b: &Object, a: &Object, b: &Object, clearance: f64) -> bool {
        if self.split_fragments.contains_key(&old_b.name) {
            return retained_split_joint(&old_a.contour, &old_b.contour, &a.contour, &b.contour, clearance);
        }
        if self.split_fragments.contains_key(&old_a.name) {
            return retained_split_joint(&old_b.contour, &old_a.contour, &b.contour, &a.contour, clearance);
        }
        retained_joint(&old_a.contour, &old_b.contour, &a.contour, &b.contour, clearance)
    }

    /// A manually submitted replacement may meet the first route leg at its
    /// original endpoint. Neither leg may double back or follow the other outside
    /// that pickup's clearance neighborhood.
    fn preserves_pickup(&self, source: &ClearanceQuery, original: usize, moved: &PadContour, incoming: &PadContour, clearance: f64) -> bool {
        let Some((pickup, layer, _)) = self.origin else { return false };
        let source = &source.objects()[original];
        let Some([a, b]) = source.contour.track_centerline() else { return false };
        if !source.layers.contains(&layer) || (a != pickup && b != pickup) {
            return false;
        }
        let opposite = |contour: &PadContour| match contour.track_centerline()? {
            [a, b] if a == pickup => Some(b),
            [a, b] if b == pickup => Some(a),
            _ => None,
        };
        let (Some(a), Some(b)) = (opposite(moved), opposite(incoming)) else { return false };
        let joint = PadContour::capsule(pickup, pickup, 0.);
        !super::chains::backtracks(a, pickup, b)
            && local_departure(moved, incoming, &joint, clearance)
            && local_departure(incoming, moved, &joint, clearance)
    }

    /// Final contours retain their original island/membership; only genuinely new route
    /// fragments inherit the original pickup contact, never the contacts of a speculative path.
    fn with_source_intent(&mut self, source: &ClearanceQuery) -> Result<ClearanceQuery, CommandError> {
        let by_name: BTreeMap<_, _> = source.objects().iter().enumerate().map(|(i, o)| (o.name.as_str(), (i, o))).collect();
        let contacts: Vec<_> = if let Some((at, layer, width)) = self.origin {
            let pickup = PadContour::segment(at, at, width)?;
            source.candidates(&pickup, &[layer])?.into_iter().filter(|i| source.objects()[*i].contour.touches(&pickup)).collect()
        } else {
            Vec::new()
        };
        let nets: Vec<_> =
            contacts.iter().flat_map(|i| source.objects()[*i].intended_nets.iter().copied()).collect::<BTreeSet<_>>().into_iter().collect();
        let island = contacts.first().map_or(usize::MAX, |i| source.objects()[*i].island);
        let mut items = self.final_geometry.items.clone();
        for item in &mut items {
            let id = Uuid::parse_str(&item.name).ok();
            let original = id.and_then(|id| self.origins.get(&id)).map(Uuid::to_string);
            let inherited = by_name.get(original.as_deref().unwrap_or(&item.name));
            self.final_displaced.push(id.is_some_and(|id| self.origins.contains_key(&id) || self.joints.contains(&id)));
            if let Some((index, original)) = inherited {
                self.final_origins.push(Some(*index));
                item.intended_nets = original.intended_nets.clone();
                item.island = original.island;
            } else {
                self.final_origins.push(None);
                item.intended_nets = nets.clone();
                item.island = island;
            }
        }
        source.with_objects(items)
    }
}

/// An unmodified fragment or subset preserves the clearance of the original pair.
fn within_source(shape: &PadContour, original: &PadContour) -> bool {
    match (shape.track_centerline(), original.track_centerline()) {
        (Some(leg), Some([a, b])) => leg.into_iter().all(|point| super::chains::on_leg(point, a, b)),
        _ => shape.core == original.core && shape.radius == original.radius,
    }
}

/// Keep only the part of the original centerline that touches the other original
/// contour. This excludes long nearby spans after a shared endpoint.
fn within_contact(shape: &PadContour, other: &PadContour, original: &PadContour, old_other: &PadContour, clearance: f64) -> bool {
    let Some([a, b]) = original.track_centerline() else { return within_source(shape, original) };
    let Some((start, end)) = old_other.track_proximity([a, b], original.radius) else { return false };
    let at =
        |t: f64| Point { x_nm: a.x_nm + ((b.x_nm - a.x_nm) as f64 * t).round() as i64, y_nm: a.y_nm + ((b.y_nm - a.y_nm) as f64 * t).round() as i64 };
    local_departure(shape, other, &PadContour::capsule(at(start), at(end), 0.), clearance)
}

/// The clearance interval on a leg must fit inside the contact region, with
/// room for the normal departure of a 45-degree turn.
fn local_departure(shape: &PadContour, other: &PadContour, contact: &PadContour, clearance: f64) -> bool {
    let Some(leg) = shape.track_centerline() else { return false };
    let Some((start, end)) = other.track_proximity(leg, shape.radius + clearance) else { return true };
    let reach = (shape.radius + other.radius + clearance) * std::f64::consts::SQRT_2;
    contact.track_proximity(leg, reach).is_some_and(|(lo, hi)| lo <= start + 1e-12 && end <= hi + 1e-12)
}

/// Bounded additive command schema; an empty list requests strict compound-route validation.
pub(crate) fn schema() -> Value {
    json!({"type":"array","maxItems":MAX_TRACKS,"items":{"type":"object","properties":{
        "track":{"type":"string","format":"uuid"},"points":path_schema()
    },"required":["track","points"],"additionalProperties":false}})
}

/// A translated through via keeps its identity, span, net assignment, diameter and drill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViaMove {
    pub via: Uuid,
    pub at: Point,
}

/// Decode source-addressed via motion before applying any part of the compound edit.
pub(crate) fn via_moves(value: &Value) -> Result<Vec<ViaMove>, CommandError> {
    if value.as_array().is_none_or(|v| v.len() > MAX_VIAS) {
        return Err(CommandError::InvalidParams("Via displacements exceed the board capacity.".into()));
    }
    let moves: Vec<ViaMove> = serde_json::from_value(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
    let mut ids = BTreeSet::new();
    if moves.iter().any(|m| m.via.is_nil() || !ids.insert(m.via) || !crate::wire::valid_point(&m.at)) {
        return Err(CommandError::InvalidParams("Each via needs a unique UUID and a bounded center.".into()));
    }
    Ok(moves)
}

/// Publish the same bounded, source-addressed rows accepted by confirmation.
pub(crate) fn via_moves_schema() -> Value {
    json!({"type":"array","maxItems":MAX_VIAS,"items":{"type":"object","properties":{"via":{"type":"string","format":"uuid"},"at":crate::point_schema()},"required":["via","at"],"additionalProperties":false}})
}

/// A directly translated joint may keep its old contact only at corresponding ends.
/// A stationary pad may retain a track end inside its land. A via needs a centered track end.
pub(crate) fn retained_joint(a: &PadContour, b: &PadContour, new_a: &PadContour, new_b: &PadContour, clearance: f64) -> bool {
    if !a.touches(b) || !new_a.touches(new_b) {
        return false;
    }
    match (a.track_centerline(), b.track_centerline(), new_a.track_centerline(), new_b.track_centerline()) {
        (Some(first), Some(second), Some(moved), Some(other)) => {
            let station = follows_station(first, second, moved, other).or_else(|| follows_station(second, first, other, moved));
            station.is_some_and(|point| {
                let joint = PadContour::capsule(point, point, 0.);
                local_departure(new_a, new_b, &joint, clearance) && local_departure(new_b, new_a, &joint, clearance)
            })
        }
        (Some(line), None, Some(moved), None) => (0..2).any(|i| {
            (line[i] == b.core[0] && moved[i] == new_b.core[0] && b.core.iter().all(|p| *p == b.core[0]))
                || (line[i] == moved[i] && b.core == new_b.core && PadContour::capsule(line[i], line[i], a.radius).touches(b))
        }),
        (None, Some(_), None, Some(_)) => retained_joint(b, a, new_b, new_a, clearance),
        _ => false,
    }
}

/// A displaced chain can split around a protected contact. Verify the fixed source station
/// on the actual final fragment, including its clearance departure from the moved joint.
fn retained_split_joint(joint: &PadContour, track: &PadContour, moved: &PadContour, fragment: &PadContour, clearance: f64) -> bool {
    let (Some(old), Some(support), Some(new), Some(piece)) =
        (joint.track_centerline(), track.track_centerline(), moved.track_centerline(), fragment.track_centerline())
    else {
        return false;
    };
    let on_leg = |point, line: [Point; 2]| super::chains::on_leg(point, line[0], line[1]);
    let at_source_end = (0..2).filter_map(|i| (old[i] == new[i] && on_leg(old[i], support)).then_some(old[i]));
    at_source_end.chain(follows_station(support, old, support, new)).any(|point| {
        if !on_leg(point, piece) || new.iter().any(|a| piece.iter().any(|b| super::chains::backtracks(*a, point, *b))) {
            return false;
        }
        let contact = PadContour::capsule(point, point, 0.);
        local_departure(moved, fragment, &contact, clearance) && local_departure(fragment, moved, &contact, clearance)
    })
}

/// A branch end on another segment must follow the same longitudinal station.
/// Check displacement in widened integer arithmetic, allowing half a nanometre per axis
/// for the editor's interpolation to the nearest lattice point. A branch cannot fold back
/// along the supporting segment after it moves.
fn follows_station(branch: [Point; 2], support: [Point; 2], moved: [Point; 2], new_support: [Point; 2]) -> Option<Point> {
    let delta = |a: i64, b: i64| i128::from(a) - i128::from(b);
    let dx = delta(support[1].x_nm, support[0].x_nm);
    let dy = delta(support[1].y_nm, support[0].y_nm);
    let length = dx * dx + dy * dy;
    (0..2)
        .find(|&i| {
            if !super::chains::on_leg(branch[i], support[0], support[1]) {
                return false;
            }
            let projection = delta(branch[i].x_nm, support[0].x_nm) * dx + delta(branch[i].y_nm, support[0].y_nm) * dy;
            let follows = |a: i64, b: i64, x: i64| 2 * (delta(x, a) * length - projection * delta(b, a)).abs() <= length;
            follows(new_support[0].x_nm, new_support[1].x_nm, moved[i].x_nm)
                && follows(new_support[0].y_nm, new_support[1].y_nm, moved[i].y_nm)
                && new_support.iter().all(|p| !super::chains::backtracks(moved[1 - i], moved[i], *p))
        })
        .map(|i| moved[i])
}

/// Validate the dimensions that a moved joint retains against the resolved rules on its layers.
pub(crate) fn joint_sizes(view: BoardView<'_>, id: Uuid, contour: &PadContour, rules: crate::board::rules::Values) -> bool {
    if contour.track_centerline().is_some() {
        return contour.radius * 2. >= rules.min_track_width_nm as f64;
    }
    view.vias().binary_search_by_key(&id, |via| via.id).ok().is_some_and(|index| {
        let via = &view.vias()[index];
        rules.validate_sizes(rules.min_track_width_nm, via.diameter_nm, via.drill_nm).is_ok()
    })
}

#[cfg(test)]
mod tests;
