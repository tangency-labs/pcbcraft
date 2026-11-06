//! Reshaping an existing wire in place: sliding a straight leg, moving an interior corner, or
//! moving a free endpoint or junction with every route attached to it.
//!
//! These edits change geometry only. Every endpoint keeps its identity, so connections and
//! component pins are never changed; a route end that is attached to something stays put and
//! gains an orthogonal leg when the moved part leaves it. Routes stay horizontal and vertical,
//! with repeated and redundant straight-through points removed. Healing overlapping or
//! doubled-back runs is not done here.

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
    /// direction. Its ends follow; the wire's own endpoints stay put.
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
    /// Build the result of moving the grabbed point of `edit` to `to`, for a drag preview or a
    /// command, without changing this snapshot. A segment moves only across its direction, so
    /// `to` must keep `at`'s coordinate along the leg. A corner must be an interior corner, not
    /// a route end. Every endpoint keeps its identity, so connectivity is unchanged. Moving to
    /// the grabbed point returns an identical project. Missing wires or junctions, points off
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
                        if along {
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
                let bends = bends_of(&rebuilt(&path, &moved, [true, true]))?;
                Arc::make_mut(&mut next.wires)[index].bends = Some(bends);
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
                    Arc::make_mut(&mut next.wires)[index].bends = Some(bends);
                }
                Arc::make_mut(&mut next.junctions).insert(*id, to);
            }
        }
        next.connectivity = Arc::default();
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
    /// step. A move that changes nothing leaves the revision and history alone.
    fn reshape(&mut self, params: &Value, edit: &Reshape) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let to = point_param(params, "to")?;
        let project = self.open_project()?;
        let next = project.reshaped(edit, to)?;
        if next.wires != project.wires || next.junctions != project.junctions {
            self.change(next);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
