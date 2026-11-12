//! Reshaping an existing wire in place: sliding a straight leg, moving an interior corner, or
//! moving a free endpoint or junction with every route attached to it.
//!
//! The pure `Project::reshaped` candidate changes geometry only. Every endpoint keeps its identity, so connections and
//! component pins are never changed; a route end that is attached to something stays put and
//! gains an orthogonal leg when the moved part leaves it. A straight one-leg piece carries its
//! degree-one free ends when slid. Routes stay horizontal and vertical,
//! with repeated and redundant straight-through points removed. Healing overlapping or
//! doubled-back runs is performed by the command through `Project::healed`.
//! Explicit attachment is a separate edit: it reshapes a junction's routes to an existing
//! endpoint or wire interior and normalizes the resulting graph.

use std::sync::Arc;

use serde_json::Value;

use super::{MAX_BENDS, OnPath, locate, valid_point};
use crate::{
    CommandError, Project, Session,
    schematic::{Point, WireEndpoint, orthogonal_path, wiring::parse_id},
};

/// One in-place geometry edit, named by what was grabbed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reshape {
    /// The straight leg of wire `wire` strictly containing `at`, which slides across its
    /// direction unless both ends of its straight one-leg wire are free, which allows
    /// translation in both axes. Interior corners follow; pins and shared junctions stay anchored.
    Segment { wire: String, at: Point },
    /// The interior corner of wire `wire` at `at`. The two legs meeting there slide with it.
    Corner { wire: String, at: Point },
    /// A free endpoint or junction, by its numeric ID. The end leg of every wire attached to it
    /// slides to keep its direction; the other end of each wire stays put.
    Junction(u64),
}

impl Reshape {
    /// The point that was grabbed: the point on the segment or corner, or the junction's
    /// position. `None` when the junction no longer exists.
    pub fn origin(&self, project: &Project) -> Option<Point> {
        match self {
            Self::Segment { at, .. } | Self::Corner { at, .. } => Some(*at),
            Self::Junction(id) => project.endpoint_position(WireEndpoint::Junction(*id)),
        }
    }
}

/// Which way a leg runs; legs are never zero-length, so one of these always holds.
fn horizontal(a: Point, b: Point) -> bool {
    a.y_nm == b.y_nm
}

/// Rebuild a route after moving some of its points to `moved` positions, then make it
/// orthogonal and drop repeated and straight-through points. When a route end is moved but
/// `fixed`, it stays where it is and the new position becomes the next corner instead.
fn rebuilt(path: &[Point], moved: &[(usize, Point)], fixed: [bool; 2]) -> Vec<Point> {
    let last = path.len().saturating_sub(1);
    let mut points = Vec::with_capacity(path.len() + 2);
    for (index, &point) in path.iter().enumerate() {
        match moved.iter().find(|(k, _)| *k == index).map(|(_, to)| *to) {
            Some(to) if index == 0 && fixed[0] => points.extend([point, to]),
            Some(to) if index == last && fixed[1] => points.extend([to, point]),
            Some(to) => points.push(to),
            None => points.push(point),
        }
    }
    orthogonal_path(points, true)
}

/// Check a rebuilt route and return its bends: at least one leg, every point in bounds and at
/// most [`MAX_BENDS`] bends.
fn bends_of(path: &[Point]) -> Result<Vec<Point>, CommandError> {
    if path.len() < 2 {
        return Err(CommandError::InvalidParams("that move would collapse the wire to a point".to_owned()));
    }
    if !path.iter().all(valid_point) {
        return Err(CommandError::InvalidParams("move exceeds schematic coordinate bounds".to_owned()));
    }
    if path.len() - 2 > MAX_BENDS {
        return Err(CommandError::Unavailable(format!("the wire would have more than {MAX_BENDS} bends")));
    }
    Ok(path[1..path.len() - 1].to_vec())
}

/// The position a neighbouring point takes so the leg between it and a point moved to `to`
/// keeps its direction: a horizontal leg follows `to` vertically, a vertical one horizontally.
fn follow(neighbour: Point, old: Point, to: Point) -> Point {
    if horizontal(neighbour, old) { Point { x_nm: neighbour.x_nm, y_nm: to.y_nm } } else { Point { x_nm: to.x_nm, y_nm: neighbour.y_nm } }
}

impl Project {
    /// Whether a straight wire has two degree-one free ends and can translate in both axes.
    /// Pins, shared junctions, bent routes and missing IDs retain constrained reshape behavior.
    pub fn wire_moves_freely(&self, wire: &str) -> bool {
        let Ok(index) = self.wire_index(wire) else {
            return false;
        };
        let wire = &self.wires[index];
        self.wire_path(wire).len() == 2
            && wire
                .endpoints
                .iter()
                .all(|end| matches!(end, WireEndpoint::Junction(_)) && self.wires.iter().filter(|wire| wire.endpoints.contains(end)).count() == 1)
    }

    /// Build the result of moving the grabbed point of `edit` to `to`, for a drag preview or a
    /// command, without changing this snapshot. A straight one-leg piece carries degree-one
    /// free ends with it; pins, shared junctions and ends of longer routes remain anchored.
    /// With two free ends the whole straight piece translates in both axes; other segments
    /// move only across their direction, so `to` keeps `at`'s coordinate along the leg. A corner must be an interior corner, not
    /// a route end. Every endpoint keeps its identity, so connectivity is unchanged. Moving to
    /// the grabbed point is checked like any other move, then returns this project unchanged,
    /// even for an automatic route. Missing wires or junctions, points off
    /// the route, results out of bounds and routes over [`MAX_BENDS`] are refused.
    pub fn reshaped(&self, edit: &Reshape, to: Point) -> Result<Project, CommandError> {
        if !valid_point(&to) {
            return Err(CommandError::InvalidParams("move exceeds schematic coordinate bounds".to_owned()));
        }
        let mut next = self.clone();
        match edit {
            Reshape::Segment { wire, at } | Reshape::Corner { wire, at } => {
                let index = self.wire_index(wire)?;
                let path = self.wire_path(&self.wires[index]);
                if path.len() < 2 {
                    return Err(CommandError::Unavailable("the wire's route can't be resolved".to_owned()));
                }
                let moved = match (edit, locate(&path, *at)?) {
                    (Reshape::Segment { .. }, OnPath::Leg(leg)) => {
                        let (a, b) = (path[leg], path[leg + 1]);
                        let along = if horizontal(a, b) { to.x_nm != at.x_nm } else { to.y_nm != at.y_nm };
                        if along && !self.wire_moves_freely(wire) {
                            return Err(CommandError::InvalidParams("a segment slides only across its direction".to_owned()));
                        }
                        let shift = |p: Point| Point { x_nm: p.x_nm + to.x_nm - at.x_nm, y_nm: p.y_nm + to.y_nm - at.y_nm };
                        vec![(leg, shift(a)), (leg + 1, shift(b))]
                    }
                    (Reshape::Segment { .. }, OnPath::Corner(_)) => {
                        return Err(CommandError::InvalidParams("that point is a corner; choose a point inside one leg".to_owned()));
                    }
                    (_, OnPath::Corner(k)) if k == 0 || k + 1 == path.len() => {
                        return Err(CommandError::InvalidParams("that point is an end of the wire, not a corner".to_owned()));
                    }
                    (_, OnPath::Corner(k)) => vec![(k - 1, follow(path[k - 1], path[k], to)), (k, to), (k + 1, follow(path[k + 1], path[k], to))],
                    (_, OnPath::Leg(_)) => return Err(CommandError::InvalidParams("that point is not a corner of the wire".to_owned())),
                };
                let mut fixed = [true, true];
                // A detached straight piece translates its free ends. Pins and junctions
                // shared with other wires remain anchored, as do ends of longer routes.
                if path.len() == 2 && matches!(edit, Reshape::Segment { .. }) {
                    for (end, endpoint) in self.wires[index].endpoints.iter().enumerate() {
                        if let WireEndpoint::Junction(id) = endpoint
                            && self.wires.iter().filter(|wire| wire.endpoints.contains(endpoint)).count() == 1
                        {
                            fixed[end] = false;
                            Arc::make_mut(&mut next.sheet_mut().junctions).insert(*id, moved[end].1);
                        }
                    }
                }
                let bends = bends_of(&rebuilt(&path, &moved, fixed))?;
                Arc::make_mut(&mut next.sheet_mut().wires)[index].bends = Some(bends);
            }
            Reshape::Junction(id) => {
                let junction = WireEndpoint::Junction(*id);
                let old = self.endpoint_position(junction).ok_or_else(|| CommandError::InvalidParams("junction does not exist".to_owned()))?;
                for (index, wire) in self.wires.iter().enumerate() {
                    let ends = wire.endpoints.map(|end| end == junction);
                    if ends == [false, false] {
                        continue;
                    }
                    if ends == [true, true] {
                        return Err(CommandError::InvalidParams("a wire starts and ends at that junction".to_owned()));
                    }
                    let path = self.wire_path(wire);
                    if path.len() < 2 {
                        return Err(CommandError::Unavailable("the wire's route can't be resolved".to_owned()));
                    }
                    let (end, neighbour) = if ends[0] { (0, 1) } else { (path.len() - 1, path.len() - 2) };
                    // The neighbour of a one-leg wire is its other, fixed end, which gains a corner.
                    let moved = [(end, to), (neighbour, follow(path[neighbour], old, to))];
                    let fixed = if ends[0] { [false, true] } else { [true, false] };
                    let bends = bends_of(&rebuilt(&path, &moved, fixed))?;
                    Arc::make_mut(&mut next.sheet_mut().wires)[index].bends = Some(bends);
                }
                Arc::make_mut(&mut next.sheet_mut().junctions).insert(*id, to);
            }
        }
        // A move to the grabbed point is valid but changes nothing. Return this project as it
        // is, so an automatic route isn't rewritten as authored bends and no undo step is added.
        if edit.origin(self) == Some(to) {
            return Ok(self.clone());
        }
        next.sheet_mut().connectivity = Arc::default();
        Ok(next)
    }

    /// The index of the wire with this ID.
    fn wire_index(&self, wire: &str) -> Result<usize, CommandError> {
        let id = parse_id(wire, "wire-");
        self.wires.iter().position(|w| Some(w.ordinal) == id).ok_or_else(|| CommandError::InvalidParams("wire does not exist".to_owned()))
    }
}

/// Read a required bounded point parameter.
fn point_param(params: &Value, key: &str) -> Result<Point, CommandError> {
    serde_json::from_value::<Point>(params[key].clone())
        .ok()
        .filter(valid_point)
        .ok_or_else(|| CommandError::InvalidParams(format!("invalid {key} point")))
}

impl Session {
    /// Attach a junction using the same validated candidate as the canvas preview, guarded
    /// by the revision at which the gesture or agent request began. Commits one undo step.
    pub(crate) fn attach_junction(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let junction = params["junction"]
            .as_str()
            .and_then(|id| parse_id(id, "junction-"))
            .ok_or_else(|| CommandError::InvalidParams("invalid junction".to_owned()))?;
        let to = super::Anchor::parse(&params["to"]).ok_or_else(|| CommandError::InvalidParams("invalid attachment target".to_owned()))?;
        let next = self.open_project()?.attached(junction, to, self.revision(), super::auto_heal(params))?;
        self.change(next)?;
        Ok(())
    }

    /// Slide the leg of `wire` containing `at` so that point moves to `to`, as one undoable
    /// edit. See [`Project::reshaped`].
    pub(crate) fn move_wire_segment(&mut self, params: &Value) -> Result<(), CommandError> {
        let edit = Reshape::Segment { wire: params["wire"].as_str().unwrap_or_default().to_owned(), at: point_param(params, "at")? };
        self.reshape(params, &edit)
    }

    /// Move the interior corner of `wire` at `at` to `to`, as one undoable edit.
    pub(crate) fn move_wire_corner(&mut self, params: &Value) -> Result<(), CommandError> {
        let edit = Reshape::Corner { wire: params["wire"].as_str().unwrap_or_default().to_owned(), at: point_param(params, "at")? };
        self.reshape(params, &edit)
    }

    /// Move a free endpoint or junction to `to`, reshaping attached routes, as one undoable edit.
    pub(crate) fn move_junction(&mut self, params: &Value) -> Result<(), CommandError> {
        let id = params["junction"]
            .as_str()
            .and_then(|value| parse_id(value, "junction-"))
            .ok_or_else(|| CommandError::InvalidParams("invalid junction".to_owned()))?;
        self.reshape(params, &Reshape::Junction(id))
    }

    /// Check the revision, build the reshaped project and adopt it as one revision and undo
    /// step, including local healing, exact pin contacts and any explicitly requested free-end
    /// wire contacts. A no-travel move without connection requests leaves history alone.
    fn reshape(&mut self, params: &Value, edit: &Reshape) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let to = point_param(params, "to")?;
        let project = self.open_project()?;
        let connections: Vec<u64> =
            params["connect_ends"].as_array().into_iter().flatten().filter_map(|end| end.as_str().and_then(|id| parse_id(id, "junction-"))).collect();
        if let Reshape::Segment { wire, .. } = edit {
            let index = project.wire_index(wire)?;
            if connections.iter().any(|id| !project.wires[index].endpoints.contains(&WireEndpoint::Junction(*id))) {
                return Err(CommandError::InvalidParams("connection must name a free end of the moved wire".into()));
            }
        }
        let next = project.reshaped(edit, to)?.healed_with_connections(project, self.revision(), super::auto_heal(params), &connections)?;
        if next.wires != project.wires || next.junctions != project.junctions {
            self.change(next)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
